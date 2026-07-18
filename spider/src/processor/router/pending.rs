use std::{
    collections::HashMap,
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};

use spider_link::{
    link_set::{
        impls::authenticated::Authenticated, links::PinnedLink, Epoch, LinkSet, LinkSetMessage,
    },
    message::{Message, RouterMessage, UiMessage},
    Relation,
};
use tokio::{
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        watch, Semaphore,
    },
    time::Instant,
};
use tokio_util::task::JoinMap;
use tracing::{debug, trace};

use crate::error::{ProblemWrap, SpiderResult};

const CODE_TIMEOUT: Duration = Duration::from_secs(600);

pub enum AddLinkResult {
    New(Relation),
    Existing,
}

/// Manages pending connections to the spider base.
///
/// These connections have yet to be approved or blocked. Additionally, the
/// first connection to the base the requests to subscribe to the UI should be
/// automatically approved.
pub struct PendingManager {
    pending_tasks: JoinMap<Relation, SpiderResult<PendingLinkResult>>,
    pending_handles: HashMap<Relation, Sender<PendingLinkControl>>,
    approval_codes: Arc<Mutex<HashMap<String, Instant>>>,
    code_watcher: watch::Sender<()>,
    ui_permits: Arc<Semaphore>,
}

impl PendingManager {
    pub fn new() -> Self {
        Self {
            pending_tasks: JoinMap::new(),
            pending_handles: HashMap::new(),
            approval_codes: Arc::new(Mutex::new(HashMap::new())),
            code_watcher: watch::Sender::new(()),
            ui_permits: Arc::new(Semaphore::new(0)),
        }
    }

    pub fn add_ui_permit(&self) {
        self.ui_permits.add_permits(1);
    }

    pub fn add_approval_code(&mut self, code: String) {
        trace!("Adding approval code {}", code);

        let mut code_map = self.approval_codes.lock().unwrap();
        let timeout = Instant::now() + CODE_TIMEOUT;
        let old_value = code_map.insert(code, timeout);

        if old_value.is_none() {
            let _ = self.code_watcher.send(());
        }
    }

    pub fn revoke_approval_code(&mut self, code: &str) {
        let mut code_map = self.approval_codes.lock().unwrap();
        code_map.remove(code);
    }

    pub async fn add_link(&mut self, link: Authenticated) -> SpiderResult<AddLinkResult> {
        let (rel, link) = link.into_parts();

        if let Some(pending) = self.pending_handles.get(&rel) {
            trace!("Pending link set existed, adding link");
            pending
                .send(PendingLinkControl::AddLink(link))
                .await
                .wrap()?;
            Ok(AddLinkResult::Existing)
        } else {
            trace!("Creating new pending link set");
            // Build parameters for task
            let (pending, ctrl_rx) = channel(50);
            let approval_codes = self.approval_codes.clone();
            let code_watcher = self.code_watcher.subscribe();
            let ui_permits = self.ui_permits.clone();

            // Prime channel
            pending
                .send(PendingLinkControl::AddLink(link))
                .await
                .expect("channel receiver is still alive because it is in scope");

            // Spawn task
            self.pending_tasks.spawn(
                rel.clone(),
                create_pending_link(ctrl_rx, approval_codes, code_watcher, ui_permits),
            );

            trace!(
                "Approval codes has {} active codes",
                self.approval_codes.lock().unwrap().len()
            );

            self.pending_handles.insert(rel.clone(), pending);
            Ok(AddLinkResult::New(rel))
        }
    }

    pub async fn approve_connection(&mut self, rel: &Relation) -> SpiderResult {
        if let Some(pending) = self.pending_handles.get(rel) {
            pending.send(PendingLinkControl::Approve).await.wrap()?;
        }
        Ok(())
    }

    pub async fn deny_connection(&mut self, rel: &Relation) -> SpiderResult {
        if let Some(pending) = self.pending_handles.get(rel) {
            pending.send(PendingLinkControl::Deny).await.wrap()?;
        }
        Ok(())
    }

    /// Returns the result of a pending connection.
    ///
    /// Always returns the Relation. Also returns Some if the relation
    /// connected, and None if it failed for any reason
    pub(super) async fn poll(
        &mut self,
    ) -> (
        Relation,
        Option<(LinkSet<Message>, Vec<(Message, Epoch)>, Option<String>)>,
    ) {
        match self.pending_tasks.join_next().await {
            Some((key, result)) => {
                self.pending_handles.remove(&key);

                let res = match result {
                    Ok(Ok(r)) => {
                        // task success
                        match r {
                            PendingLinkResult::Approved(link_set, items, used_code) => {
                                Some((link_set, items, used_code))
                            }
                            PendingLinkResult::Denied => None,
                        }
                    }
                    // task errored
                    Ok(Err(e)) => {
                        debug!("Pending connection task errored: {}", e);
                        None
                    }
                    // task panicked or aborted
                    Err(e) => {
                        debug!("Pending connection task panicked or aborted: {}", e);
                        None
                    }
                };

                return (key, res);
            }
            None => pending().await,
        }
    }

    /// Returns the list of expired codes for removal from the UI
    pub fn upkeep(&mut self) -> Vec<String> {
        // Clean approval codes
        self.approval_codes
            .lock()
            .unwrap()
            .extract_if(|_, timeout| timeout < &mut Instant::now())
            .map(|(c, _)| c)
            .collect()
    }
}

// Pending Link Processor functions

pub enum PendingLinkControl {
    Approve,
    Deny,
    AddLink(Box<dyn PinnedLink>),
}

pub enum PendingLinkResult {
    Approved(LinkSet<Message>, Vec<(Message, Epoch)>, Option<String>),
    Denied,
}

async fn create_pending_link(
    mut ctrl_rx: Receiver<PendingLinkControl>,
    approval_codes: Arc<Mutex<HashMap<String, Instant>>>,
    mut code_watcher: watch::Receiver<()>,
    ui_permits: Arc<Semaphore>,
) -> SpiderResult<PendingLinkResult> {
    let mut link_set = LinkSet::<Message>::new();
    link_set
        .set_grace_period_timeout(Some(Duration::from_secs(30)))
        .await
        .wrap()?;

    let mut backlog = Vec::new();
    let mut recvd_code = Option::<String>::None;
    let mut code_attempts = 0;
    let mut recvd_ui_sub = false;
    loop {
        select! {
            msg = ctrl_rx.recv() => {
                let Some(msg) = msg else {break;};
                // handle control message
                match msg {
                    PendingLinkControl::Approve => {
                        return Ok(PendingLinkResult::Approved(link_set, backlog, None));
                    },
                    PendingLinkControl::Deny => break,
                    PendingLinkControl::AddLink(link) => {
                        trace!("Pending adding new link");
                        link_set.add_link_boxed(link).await.wrap()?;
                    },
                }

            },
            res = code_watcher.changed() => {
                // If the watcher is closed, the pending manager must have been
                // dropped. Nothing left to do.
                res.wrap()?;
                if let Some(recvd_code) = recvd_code.as_ref() {
                    if let Some((used_code, _)) = approval_codes.lock().unwrap().remove_entry(recvd_code) {
                        return Ok(PendingLinkResult::Approved(link_set, backlog, Some(used_code)));
                    }
                }
            },
            msg = link_set.recv() => {
                let Ok(msg) = msg else {break;};
                // handle link set message
                match msg {
                    LinkSetMessage::Disconnected => break,
                    LinkSetMessage::Connected(_) => {
                        link_set.send(Message::Router(RouterMessage::Pending)).await.wrap()?;
                    },
                    LinkSetMessage::AttemptingConnection(_) => {} // Base's link sets do not have a way to acquire more addresses.
                    LinkSetMessage::Message(message, epoch) => {
                        trace!("Pending link set got message");
                        // check messages for incoming approval codes
                        if let Message::Router(RouterMessage::ApprovalCode(new_code)) = &message {
                            let mut codes = approval_codes.lock().unwrap();
                            trace!("Pending link set got approval code {}, {} #codes in set", new_code, codes.len());
                            if let Some((used_code, _)) = codes.remove_entry(new_code) {
                                trace!("Matched code, sending ApprovedConnection");
                                return Ok(PendingLinkResult::Approved(link_set, backlog, Some(used_code)));
                            }else{
                                trace!("Mismatched code");
                                recvd_code = Some(new_code.clone());
                                code_attempts += 1;
                                // too many attempts, deny
                                if code_attempts > 5 {
                                    trace!("Too many mismatched attempts, canceling");
                                    break;
                                }
                            }
                        }

                        // check messages for incoming ui subscription messages
                        if let Message::Ui(UiMessage::Subscribe) = &message {
                            match ui_permits.try_acquire() {
                                Ok(permit) => {
                                    // have a permit and received the ui
                                    // message, accept this connection
                                    backlog.push((message, epoch));
                                    permit.forget();
                                    return Ok(PendingLinkResult::Approved(link_set, backlog, None));
                                },
                                Err(_) => {
                                    // couldn't get permit now, try later
                                    recvd_ui_sub = true;
                                },
                            }
                        }

                        backlog.push((message, epoch));
                        if backlog.len() > 100 {
                            // too many messages in backlog
                            break;
                        }
                    },
                }
            },
            Ok(permit) = ui_permits.acquire(), if recvd_ui_sub && !ui_permits.is_closed() => {
                permit.forget();
                return Ok(PendingLinkResult::Approved(link_set, backlog, None));
            }
        }
    }
    // if we break from the loop, assume that the link should be denied
    let _ = link_set.send(Message::Router(RouterMessage::Denied)).await;
    Ok(PendingLinkResult::Denied)
}

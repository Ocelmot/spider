use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use tracing::{info, trace};
use spider_link::{ Relation, SelfRelation, link_set::{LinkSet, LinkSetMessage, impls::authenticated::Authenticated, links::PinnedLink}, message::{Message, RouterMessage, UiMessage}};
use tokio::{
    select, spawn,
    sync::{
        mpsc::{channel, Sender},
        Semaphore,
    },
    time::Instant,
};

use crate::processor::{link::ProcessorLink, message::ProcessorMessage, ui::UiProcessorMessage};

use super::RouterProcessorMessage;

const CODE_TIMEOUT: Duration = Duration::from_secs(600);

/// Manages pending connections to the spider base.
///
/// These connections have yet to be approved or blocked. Additionally, the
/// first connection to the base the requests to subscribe to the UI should be
/// automatically approved.
pub struct PendingManager {
    pl: ProcessorLink,
    sender: Sender<RouterProcessorMessage>,
    pending_connections: HashMap<Relation, Sender<PendingLinkControl>>,
    approval_codes: HashMap<String, Instant>,
    ui_permits: Arc<Semaphore>,
}

impl PendingManager {
    pub fn new(pl: ProcessorLink, sender: Sender<RouterProcessorMessage>) -> Self {
        Self {
            pl,
            sender,
            pending_connections: HashMap::new(),
            approval_codes: HashMap::new(),
            ui_permits: Arc::new(Semaphore::new(0)),
        }
    }

    pub fn add_ui_permit(&self) {
        self.ui_permits.add_permits(1);
    }

    pub async fn add_approval_code(&mut self, code: String) {
        trace!("Adding approval code {}", code);
        for (_, pending) in &self.pending_connections {
            pending.send(PendingLinkControl::AddCode(code.clone())).await;
        }

        let timeout = Instant::now() + CODE_TIMEOUT;
        self.approval_codes.insert(code, timeout);
    }

    pub async fn revoke_approval_code(&mut self, code: String) {
        for (_, pending) in &self.pending_connections {
            pending.send(PendingLinkControl::RevokeCode(code.clone())).await;
        }

        self.approval_codes.remove(&code);
    }

    pub async fn add_link(&mut self, link: Authenticated) {
        let (rel, link) = link.into_parts();
        if let Some(pending) = self.pending_connections.get(&rel) {
            trace!("Pending link set existed, adding link");
            pending.send(PendingLinkControl::AddLink(link)).await;
        } else {
            trace!("Creating new pending link set");
            let sender = self.sender.clone();
            let self_rel = self.pl.state().self_relation().await;
            let ui_permits = self.ui_permits.clone();
            let pending = create_pending_link(sender, self_rel, rel.clone(), ui_permits);

            trace!("Approval codes has {} active codes", self.approval_codes.len());
            for (code, _) in &self.approval_codes {
                trace!("Adding approval code {}", code );
                pending.send(PendingLinkControl::AddCode(code.clone())).await;
            }

            pending.send(PendingLinkControl::AddLink(link)).await;

            add_pending_ui_setting(&self.pl, &rel).await;
            self.pending_connections.insert(rel, pending);
        }
    }

    pub async fn approve_connection(&mut self, rel: &Relation) {
        if let Some(pending) = self.pending_connections.remove(rel) {
            pending.send(PendingLinkControl::Approve).await;
        }
        remove_pending_ui_setting(&self.pl, rel).await;
    }
    

    pub async fn deny_connection(&mut self, rel: &Relation) {
        if let Some(pending) = self.pending_connections.remove(rel) {
            pending.send(PendingLinkControl::Deny).await;
        }
        remove_pending_ui_setting(&self.pl, rel).await;
    }

    pub(super) async fn remove_connection(&mut self, rel: &Relation){
        self.pending_connections.remove(rel);
        remove_pending_ui_setting(&self.pl, rel).await;
    }

    pub fn upkeep(&mut self) {
        // Clean approval codes
        self.approval_codes.retain(|_, timeout| timeout >= &mut Instant::now());
    }
}

// Ui Helper functions

async fn add_pending_ui_setting(pl: &ProcessorLink, rel: &Relation) {
    let sig= rel.sig();
    let title = format!("{:?}: {}", rel.role, sig);
    let msg = UiProcessorMessage::SetSetting {
        header: String::from("Pending Connections"),
        title,
        inputs: vec![
            ("button".to_string(), "Approve".to_string()),
            ("button".to_string(), "Deny".to_string()),
        ],
        cb: |e| {
            if e.index() == 0 {
                // Approve
                if let Some(rel) = Relation::from_base64(e.data()){
                    let msg = RouterProcessorMessage::ApproveConnection(rel);
                    return Some(ProcessorMessage::RouterMessage(msg));
                }
            }
            if e.index() == 1 {
                // Deny
                if let Some(rel) = Relation::from_base64(e.data()) {
                    let msg = RouterProcessorMessage::DenyConnection(rel);
                    return Some(ProcessorMessage::RouterMessage(msg));
                }
            }
            None
        },
        data: rel.to_base64(),
    };
    pl.send_ui(msg).await;
}

pub async fn remove_pending_ui_setting(pl: &ProcessorLink, rel: &Relation) {
    let sig = rel.sig();
    let title = format!("{:?}: {}", rel.role, sig);
    let msg = UiProcessorMessage::RemoveSetting {
        header: String::from("Pending Connections"),
        title,
    };
    pl.send_ui(msg).await;
}

// Pending Link Processor functions

pub enum PendingLinkControl {
    Approve,
    Deny,
    AddCode(String),
    RevokeCode(String),
    AddLink(Box<dyn PinnedLink>),
}



fn create_pending_link(sender: Sender<RouterProcessorMessage>, self_rel: SelfRelation, rel: Relation, ui_permits: Arc<Semaphore>) -> Sender<PendingLinkControl> {
    let (ctrl_tx, mut ctrl_rx) = channel(50);
    
    spawn(async move {
        let mut link_set = LinkSet::<Message>::new();
        link_set.set_grace_period_timeout(Some(Duration::from_secs(30))).await;

        let mut backlog = Vec::new();
        let mut codes = HashSet::new();
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
                            let msg = RouterProcessorMessage::ApprovedConnection(rel.clone(), backlog, link_set);
                            sender.send(msg).await;
                            return;
                        },
                        PendingLinkControl::Deny => break,
                        PendingLinkControl::AddCode(code) => {
                            trace!("Pending link got new code: {}", code);
                            if let Some(recvd_code) = &recvd_code {
                                trace!("Checking vs recvd_code");
                                if code == *recvd_code {
                                    trace!("Matched");
                                    let msg = RouterProcessorMessage::ApprovedConnection(rel.clone(), backlog, link_set);
                                    sender.send(msg).await;
                                    return; 
                                }else{
                                    trace!("didn't match");
                                }
                            }
                            codes.insert(code);
                        },
                        PendingLinkControl::RevokeCode(code) => {
                            codes.remove(&code);
                        }
                        PendingLinkControl::AddLink(link) => {
                            trace!("Pending adding new link");
                            link_set.add_link_boxed(link).await;
                        },
                    }

                },
                msg = link_set.recv() => {
                    let Ok(msg) = msg else {break;};
                    // handle link set message
                    match msg {
                        LinkSetMessage::Disconnected => break,
                        LinkSetMessage::Connected(_) => {
                            link_set.send(Message::Router(RouterMessage::Pending)).await;
                        },
                        LinkSetMessage::AttemptingConnection(_) => {} // Base's link sets do not have a way to acquire more addresses.
                        LinkSetMessage::Message(message, epoch) => {
                            trace!("Pending link set got message");
                            // check messages for incoming approval codes
                            if let Message::Router(RouterMessage::ApprovalCode(new_code)) = &message {
                                trace!("Pending link set got approval code {}, {} #codes in set", new_code, codes.len());
                                if codes.contains(new_code) {
                                    trace!("Matched code, sending ApprovedConnection");
                                    let msg = RouterProcessorMessage::ApprovedConnection(rel.clone(), backlog, link_set);
                                    sender.send(msg).await;
                                    return;
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
                                        let msg = RouterProcessorMessage::ApprovedConnection(rel.clone(), backlog, link_set);
                                        sender.send(msg).await;
                                        permit.forget();
                                        return;
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
                    let msg = RouterProcessorMessage::ApprovedConnection(rel.clone(), backlog, link_set);
                    sender.send(msg).await;
                    permit.forget();
                    return;
                }
            }
        }
        // if we break from the loop, assume that the link should be denied
        sender.send(RouterProcessorMessage::DenyConnection(rel.clone())).await;
    });

    ctrl_tx
}

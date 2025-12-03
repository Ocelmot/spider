use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use directory::Directory;
use pending::PendingManager;
use spider_link::{
    Relation, identified_link::IdentifiedLink, link_set::{Epoch, LinkSet, LinkSetMessage, impls::TCPLink, links::PinnedLink}, message::{Invite, Message, RouterMessage}
};
use tokio::{
    select,
    sync::{
        mpsc::{channel, error::SendError, Receiver, Sender},
        Mutex,
    },
    task::{JoinError, JoinHandle},
};
use tracing::{debug, info};

use rand::{
    distributions::Alphanumeric, rngs::StdRng, seq::SliceRandom, thread_rng, Rng, SeedableRng,
};
use tracing::trace;

use crate::error::{ProblemWrap, SpiderError, SpiderResult};

use super::{link::ProcessorLink, message::ProcessorMessage, ui::UiProcessorMessage};

mod pending;

mod directory;
mod event;

mod message;
pub use message::RouterProcessorMessage;

pub(crate) struct RouterProcessor {
    sender: Sender<RouterProcessorMessage>,
    handle: JoinHandle<()>,
}

impl RouterProcessor {
    pub async fn new(pl: ProcessorLink) -> SpiderResult<Self> {
        let (router_sender, router_receiver) = channel(50);
        let processor =
            RouterProcessorState::new(pl, router_sender.clone(), router_receiver).await?;
        let handle = processor.start();
        Ok(Self {
            sender: router_sender,
            handle,
        })
    }

    pub async fn send(
        &mut self,
        message: RouterProcessorMessage,
    ) -> Result<(), SendError<RouterProcessorMessage>> {
        self.sender.send(message).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

pub(crate) struct RouterProcessorState {
    pl: ProcessorLink,
    sender: Sender<RouterProcessorMessage>,
    receiver: Receiver<RouterProcessorMessage>,

    key_req: Arc<Mutex<Option<String>>>,
    listeners: Receiver<Box<dyn IdentifiedLink>>,

    // Directory
    directory: Directory,

    // Pending Link items
    pending: PendingManager,

    /// Current LinkSets
    links: HashMap<Relation, LinkSet<Message>>,

    // Event items
    event_subscribers: HashMap<String, HashSet<Relation>>,
    // veilid: VeilidHub,
}

impl RouterProcessorState {
    pub async fn new(
        pl: ProcessorLink,
        sender: Sender<RouterProcessorMessage>,
        receiver: Receiver<RouterProcessorMessage>,
    ) -> SpiderResult<Self> {
        let directory = Directory::load_directory(pl.clone()).await;
        let pending = PendingManager::new(pl.clone(), sender.clone());

        if directory.is_empty() {
            info!("Directory empty, adding a UI Permit");
            pending.add_ui_permit();
        }

        let (listen_tx, listen_rx) = channel::<Box<dyn IdentifiedLink>>(10);

        // set up tcp listener
        let name = pl.state().name().await.clone();
        let key_req = if pl.config().key_req_enabled() {
            info!("Key requests enabled, current name = {name}");
            Arc::new(Mutex::new(Some(name)))
        } else {
            info!("Key requests disabled");
            Arc::new(Mutex::new(None))
        };
        let self_relation = pl.state().self_relation().await;
        let listen_addr = pl.config().listen_addr.clone();
        let mut tcp_listener = TCPLink::listen_key_req(self_relation, listen_addr, key_req.clone());
        let task_tx = listen_tx.clone();
        tokio::spawn(async move {
            loop {
                match tcp_listener.recv().await {
                    Some(link) => task_tx.send(Box::new(link)).await,
                    None => break,
                };
            }
        });

        Ok(Self {
            pl,
            sender,
            receiver,

            key_req,
            listeners: listen_rx,

            directory,

            pending,

            links: HashMap::new(),

            event_subscribers: HashMap::new(),
            // veilid,
        })
    }

    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            self.init_ui().await;
            loop {
                select! {
                    Some(link) = self.listeners.recv(), if !self.listeners.is_closed() => {
                        trace!("RouterProcessor got new link");
                        match self.directory.is_link_approved(link.other_relation()) {
                            directory::LinkApproval::Blocked => {
                                // do nothing, close the link
                                trace!("Link blocked")
                            },
                            directory::LinkApproval::Unknown => {
                                // add link to pending
                                trace!("Link Unknown");
                                self.pending.add_link(link).await;
                            },
                            directory::LinkApproval::Allowed => {
                                // insert into existing link set, or create new
                                // link set.
                                trace!("Link allowed");
                                self.insert_link(link).await;
                            },
                        }
                    }
                    msg = self.receiver.recv() => {
                        let Some(msg) = msg else {break};

                        self.process_message(msg).await;
                    }
                }
            }
        });
        handle
    }

    async fn init_ui(&mut self) {
        // ===== Setup menu items =====
        // Change/Set name
        let name = self.pl.state().name().await.clone();
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("System"),
            title: "Name:".into(),
            inputs: vec![
                ("text".to_string(), name),
                ("textentry".to_string(), "New Name".into()),
            ],
            cb: |e| match e.input() {
                spider_link::message::UiInput::Click => None,
                spider_link::message::UiInput::Text(name) => {
                    let router_msg = RouterProcessorMessage::SetName(name.clone());
                    let msg = ProcessorMessage::RouterMessage(router_msg);
                    Some(msg)
                }
            },
            data: String::new(),
        };
        self.pl.send_ui(msg).await;
    }

    async fn insert_link<L>(&mut self, link: L) -> SpiderResult
    where
        L: Into<Box<dyn IdentifiedLink>> + 'static,
    {
        let link = link.into();
        let rel = link.other_relation().clone();
        match self.links.get_mut(&rel) {
            Some(link_set) => link_set.add_link(link).await.wrap(),
            None => {
                let mut link_set = self.create_link_set(rel.clone()).await?;
                link_set.add_link(link).await;

                let mut recv = link_set
                    .take_recv()
                    .ok_or(SpiderError::new().msg("could not take recv from link"))?;
                let task_channel = self.sender.clone();
                let task_rel = rel.clone();
                tokio::spawn(async move {
                    loop {
                        match recv.recv().await {
                            Ok(msg) => match msg {
                                LinkSetMessage::Disconnected => {
                                    info!("link disconnected");
                                    if task_channel
                                        .send(RouterProcessorMessage::Disconnected(
                                            task_rel.clone(),
                                        ))
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                                LinkSetMessage::Connecting(_) => {} // The base's link_sets do not have reconnect enabled
                                LinkSetMessage::Message(message, _) => {
                                    if task_channel
                                        .send(RouterProcessorMessage::UnapprovedMessage(
                                            task_rel.clone(),
                                            message,
                                        ))
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                                LinkSetMessage::Connected(epoch) => {
                                    info!("link connected with epoch {}", epoch);
                                    if task_channel
                                        .send(RouterProcessorMessage::Connected(
                                            task_rel.clone(),
                                            epoch,
                                        ))
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            },
                            Err(_) => break,
                        }
                    }
                });

                self.links.insert(rel, link_set);

                Ok(())
            }
        }
    }

    async fn process_message(&mut self, msg: RouterProcessorMessage) -> SpiderResult {
        match msg {
            RouterProcessorMessage::PeripheralMessage(rel, msg) => {
                self.process_remote_message(rel, msg).await;
            }

            // ===== Pending connection operations =====
            RouterProcessorMessage::ApproveConnection(relation) => {
                self.pending.approve_connection(&relation).await;
            }
            RouterProcessorMessage::DenyConnection(relation) => {
                self.pending.deny_connection(&relation).await;
            }
            RouterProcessorMessage::AddApprovalCode(code) => {
                self.pending.add_approval_code(code).await;
            }
            RouterProcessorMessage::ApprovedConnection(rel, backlog, link_set) => {
                self.approved_connection_handler(rel, backlog, link_set).await;
            }

            RouterProcessorMessage::Connected(rel, epoch) => {
                if let Some(link) = self.links.get(&rel) {
                    // Send our name to them
                    let name = self.pl.state().name().await.clone();
                    let msg = RouterMessage::SetIdentityProperty("name".into(), name);
                    let msg = Message::Router(msg);
                    let _ = link.send(msg).await;

                    // Send our address to them
                    // if self.pl.config().veilid_enabled() {
                    //     let dht = self.pl.state().veilid_own_dht().await;
                    //     if let Some(addr) = dht.deref() {
                    //         let addr = addr.key().to_string();
                    //         let msg = RouterMessage::Addrs(vec![addr]);
                    //         let msg = Message::Router(msg);
                    //         let _ = link.send(msg).await;
                    //     }else{
                    //         trace!("No veilid address stored");
                    //     }
                    // }else{
                    //     trace!("Veilid not enabled");
                    // }
                }
            }
            RouterProcessorMessage::UnapprovedMessage(rel, msg) => {
                if self.directory.approve_message(&rel, &msg) {
                    debug!("Approved message: {:?}", msg);
                    self.pl
                        .send(ProcessorMessage::RemoteMessage(rel, msg))
                        .await
                        .wrap()?;
                }
            }
            RouterProcessorMessage::Disconnected(_rel) => {}

            // ===== Message sending operations =====
            RouterProcessorMessage::SendMessage(rel, msg) => {
                self.send_msg(rel, msg).await;
            }
            RouterProcessorMessage::MulticastMessage(rels, msg) => {
                self.multicast_msg(rels, msg).await;
            }
            RouterProcessorMessage::SomecastMessage(rels, min, msg) => {
                self.somecast_msg(rels, min, msg).await;
            }

            RouterProcessorMessage::SetName(name) => {
                // save new name
                let mut state_name = self.pl.state().name().await;
                *state_name = name.clone();
                drop(state_name);

                // inform listener
                let mut key_req = self.key_req.lock().await;
                if let Some(old_name) = &mut *key_req {
                    *old_name = name.clone();
                }
                drop(key_req);

                // update setting
                let msg = UiProcessorMessage::SetSetting {
                    header: String::from("System"),
                    title: "Name:".into(),
                    inputs: vec![
                        ("text".to_string(), name.clone()),
                        ("textentry".to_string(), "New Name".into()),
                    ],
                    cb: |e| match e.input() {
                        spider_link::message::UiInput::Click => None,
                        spider_link::message::UiInput::Text(name) => {
                            let router_msg = RouterProcessorMessage::SetName(name.clone());
                            let msg = ProcessorMessage::RouterMessage(router_msg);
                            Some(msg)
                        }
                    },
                    data: String::new(),
                };
                self.pl.send_ui(msg).await;

                // message name on existing channels
                for (_, link) in &self.links {
                    let msg = RouterMessage::SetIdentityProperty("name".into(), name.clone());
                    let msg = Message::Router(msg);
                    link.send(msg).await;
                }
            }
            RouterProcessorMessage::SetNickname(rel, name) => {
                self.directory
                    .set_system_property(rel, "nickname", name)
                    .await;
            }
            RouterProcessorMessage::SetDirectoryEntry(rel, key, value) => {
                self.directory.set_system_property(rel, key, value).await;
            }
            RouterProcessorMessage::ClearDirectoryEntry(rel) => {
                self.directory.remove_identity(&rel).await;
            }

            RouterProcessorMessage::RevokeInvite(invite_id) => {
                self.handle_revoke_invite(invite_id).await;
            }

            RouterProcessorMessage::Upkeep => {
                // should check for disconnected peers, and clean them up

                self.directory.upkeep().await;
                self.pending.upkeep();
            }
        }

        Ok(())
    }

    async fn process_remote_message(&mut self, rel: Relation, msg: RouterMessage) {
        match msg {
            // Authorization messages
            RouterMessage::Pending => {} // base sends this, not recv
            // This message should not be recvd here, since it is only valid
            // when the link is pending and messages that arrive here
            // are already approved.
            RouterMessage::ApprovalCode(_) => {}
            RouterMessage::Approved => {} // base sends this, not recv
            RouterMessage::Denied => {}   // base sends this, not recv
            RouterMessage::Addrs(addrs) => {
                if let Ok(addrs) = serde_json::ser::to_string(&addrs) {
                    self.directory
                        .set_system_property(rel, "addrs", addrs)
                        .await;
                }
            }

            // Event Messages
            RouterMessage::SendEvent(name, externals, data) => {
                self.handle_send_event(rel.clone(), name, externals, data)
                    .await;
            }
            RouterMessage::Event(name, _, data) => {
                // re-route events from peers to appropriate peripherals
                // The known relation of the link is used as the from field in the event
                self.handle_event(name, rel, data).await;
            }
            RouterMessage::Subscribe(name) => {
                if rel.is_peer() {
                    return; // dont allow subscriptions from peers (at least for now)
                }
                let entry = self.event_subscribers.entry(name);
                let subscriber_set = entry.or_default();
                subscriber_set.insert(rel);
            }
            RouterMessage::Unsubscribe(name) => {
                if rel.is_peer() {
                    return; // dont allow subscriptions from peers (at least for now)
                }
                match self.event_subscribers.get_mut(&name) {
                    Some(subscriber_set) => {
                        subscriber_set.remove(&rel);
                        if subscriber_set.is_empty() {
                            self.event_subscribers.remove(&name);
                        }
                    }
                    None => {} // there were no subscribers to this message type
                }
            }

            // Directory Messages
            RouterMessage::SubscribeDir => {
                self.directory.add_subscriber(rel).await;
            }
            RouterMessage::UnsubscribeDir => {
                self.directory.remove_subscriber(&rel);
            }
            RouterMessage::AddIdentity(_) => {
                // base send this, doesn't receive
            }
            RouterMessage::RemoveIdentity(_) => {
                // base send this, doesn't receive
            }
            RouterMessage::SetIdentityProperty(key, value) => {
                self.directory.set_self_property(rel, key, value).await;
            }

            // Invite Messages
            RouterMessage::Invite(invite) => {
                self.handle_invite(invite).await;
            }
            RouterMessage::GenerateInvite => {
                self.handle_generate_invite(Some(rel)).await;
            }
        }
    }

    async fn approved_connection_handler(
        &mut self,
        relation: Relation,
        backlog: Vec<(Message, Epoch)>,
        mut link_set: LinkSet<Message>,
    ) {
        info!("Connection Approved");

        // get receiver + relation from link
        let mut rx = match link_set.take_recv() {
            Some(rx) => rx,
            None => return,
        };
        // let relation = link_set.other_relation().clone();

        // Inform other side that the connection is approved.
        let msg = Message::Router(RouterMessage::Approved);
        link_set.send(msg).await;

        // add link relation to directory
        self.directory.add_identity(&relation).await;

        // add link to structures
        self.links.insert(relation.clone(), link_set);

        // send backlogged messages
        for (msg, _epoch) in backlog {
            if self.directory.approve_message(&relation, &msg) {
                self.pl
                    .send(ProcessorMessage::RemoteMessage(relation.clone(), msg))
                    .await;
            }
        }

        // start link processor
        let pl = self.pl.clone();
        tokio::spawn(async move {
            loop {
                let Ok(msg) = rx.recv().await else { break };

                match msg {
                    LinkSetMessage::Disconnected => {}
                    LinkSetMessage::Connected(_) => {}
                    LinkSetMessage::Connecting(_) => {}
                    LinkSetMessage::Message(message, _) => {
                        let msg = ProcessorMessage::RemoteMessage(relation.clone(), message);
                        if pl.send(msg).await.is_err() {
                            // If processor has stopped, disconnect
                            break;
                        }
                    }
                }
            }
        });
    }

    async fn send_msg(&mut self, rel: Relation, msg: Message) -> SpiderResult {
        trace!("Sending message: {:?}", msg);

        if let Some(link_set) = self.links.get_mut(&rel) {
            if link_set.send(msg).await.is_err() {
                self.links.remove(&rel);
            }
        } else {
            let link_set = self.create_link_set(rel.clone()).await?;

            link_set.send(msg).await.wrap()?;
            self.links.insert(rel, link_set);
        }
        Ok(())
    }

    async fn multicast_msg(&mut self, relations: Vec<Relation>, msg: Message) -> SpiderResult {
        for relation in relations {
            self.send_msg(relation, msg.clone()).await?;
        }
        Ok(())
    }

    async fn somecast_msg(&mut self, relations: Vec<Relation>, min: usize, msg: Message) {
        let mut connected = Vec::new();
        let mut disconnected = Vec::new();
        for rel in relations {
            if self.links.contains_key(&rel) {
                connected.push(rel);
            } else {
                disconnected.push(rel)
            }
        }
        let mut rng = StdRng::from_rng(rand::thread_rng()).unwrap();
        if connected.len() >= min {
            for receiver in connected.choose_multiple(&mut rng, min) {
                if let Some(link) = self.links.get_mut(receiver) {
                    link.send(msg.clone()).await;
                }
            }
        } else {
            let disconnected_count = min - connected.len();
            for rel in connected {
                if let Some(link) = self.links.get_mut(&rel) {
                    link.send(msg.clone()).await;
                }
            }
            for receiver in disconnected.choose_multiple(&mut rng, disconnected_count) {
                if let Some(link) = self.links.get_mut(receiver) {
                    link.send(msg.clone()).await;
                }
            }
        }
    }

    /// handle request from peripheral to accept an invite (external message)
    async fn handle_invite(&mut self, invite: Invite) -> SpiderResult {
        if !self.links.contains_key(invite.rel()) {
            let link_set = self.create_link_set(invite.rel().clone()).await?;
            self.links.insert(invite.rel().clone(), link_set);
        }

        let link_set = self
            .links
            .get_mut(invite.rel())
            .expect("missing link should have been created");

        // add addrs from invite
        for addr in invite.addrs() {
            link_set.add_addr(addr.clone()).await;
        }

        // send approval code
        let msg = RouterMessage::ApprovalCode(invite.invite_code().clone());
        link_set.send(Message::Router(msg)).await;
        Ok(())
    }

    /// handle request from peripheral to generate an invite (external message)
    async fn handle_generate_invite(&mut self, rel: Option<Relation>) {
        info!("Generating Invite...");

        let self_rel = self.pl.state().self_relation().await;
        // could possibly also add tcp/ip addr here as well
        // let veilid_addr = self.veilid.listen_dht().key().to_string();
        // TODO: get addresses for invitation (maybe set up a way to choose which network the invitation is for (tcp, chord, etc...))
        let addrs = vec![];
        let rng = thread_rng();
        let invite_code = rng
            .sample_iter(Alphanumeric)
            .take(15)
            .map(char::from)
            .collect();
        let invite = Invite::new(&self_rel, addrs, invite_code);

        // add code to pending
        self.pending
            .add_approval_code(invite.invite_code().to_string())
            .await;

        // add invite to ui
        let msg = UiProcessorMessage::SetSetting {
            header: "Pending Connections".into(),
            title: format!("Invite: {}", invite.invite_code()),
            inputs: vec![
                ("button".into(), "View".into()),
                ("button".into(), "Revoke".into()),
            ],
            cb: |e| {
                let invite = Invite::from_base64(e.data()).expect("base 64 should parse");
                // View
                if e.index() == 0 {
                    let msg = RouterMessage::Invite(invite);
                    let msg = Message::Router(msg);
                    let msg = RouterProcessorMessage::SendMessage(e.rel().clone(), msg);
                    let msg = ProcessorMessage::RouterMessage(msg);
                    return Some(msg);
                }
                // Revoke
                else if e.index() == 1 {
                    let revoke_id = invite.invite_code().to_string();
                    let msg = RouterProcessorMessage::RevokeInvite(revoke_id);
                    let msg = ProcessorMessage::RouterMessage(msg);
                    return Some(msg);
                }
                None
            },
            data: invite.to_base64(),
        };
        self.pl.send_ui(msg).await;

        // reply with generated invite
        if let Some(rel) = rel {
            let msg = Message::Router(RouterMessage::Invite(invite));
            self.send_msg(rel, msg).await;
        }
    }

    /// handle the response from the UI button (internal message)
    pub(crate) async fn handle_revoke_invite(&mut self, invite_code: String) {
        // revoke from pending connections
        self.pending.revoke_approval_code(invite_code.clone()).await;

        // remove from UI
        let msg = UiProcessorMessage::RemoveSetting {
            header: "Pending Connections".into(),
            title: format!("Invite: {}", invite_code),
        };
        self.pl.send_ui(msg).await;
    }

    async fn create_link_set(&self, rel: Relation) -> SpiderResult<LinkSet<Message>> {
        let self_rel = self.pl.state().self_relation().await;
        let link_set = LinkSet::new();

        // add known addrs
        if let Some(addrs) = self.directory.get_system_property(&rel, "addrs") {
            let addrs = serde_json::from_str(addrs).unwrap_or(Vec::new());
            for addr in addrs {
                link_set.add_addr(addr).await.wrap()?;
            }
        };

        // add tcp link capability
        link_set
            .add_connector(move |addr| {
                let sr = self_rel.clone();
                let r = rel.clone();
                async { TCPLink::connect(sr, r, addr).await.map_err(|_| spider_link::link_set::LinkSetError::Closed) }
            })
            .await
            .wrap()?;

        // TODO: make the link capabilities variable

        // add veilid link capability
        // let connector = self.veilid.get_connector();
        // link_set
        //     .add_connector(move |_, rel, addr| {
        //         let c = connector.clone();
        //         async move { c.connect(rel, addr).await }
        //     })
        //     .await;

        Ok(link_set)
    }
}

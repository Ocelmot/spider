use std::{collections::{HashMap, HashSet}, time::Duration};

use dht_chord::{chord::ChordHandle, associate::{AssociateRequest, AssociateResponse}, TCPChord};
use lru::LruCache;
use spider_link::{
    message::{Message, RouterMessage, DirectoryEntry},
    Link, Relation, Role, SpiderId2048,
};
use tokio::{
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle}, time::Instant, select,
};

use crate::{config::SpiderConfig, state_data::StateData};

use self::{chord::ChordEntry, authorization::PendingLinkControl};

use super::{message::ProcessorMessage, sender::ProcessorSender, ui::UiProcessorMessage, listener::ListenProcessorMessage};

mod authorization;
mod event;
mod chord;
pub use chord::ChordState;
mod directory;

mod message;
pub use message::RouterProcessorMessage;

pub(crate) struct RouterProcessor {
    sender: Sender<RouterProcessorMessage>,
    handle: JoinHandle<()>,
}

impl RouterProcessor {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender) -> Self {
        let (router_sender, router_receiver) = channel(50);
        let processor = RouterProcessorState::new(config, state, sender, router_receiver);
        let handle = processor.start();
        Self {
            sender: router_sender,
            handle,
        }
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
    config: SpiderConfig,
    state: StateData,
    sender: ProcessorSender,
    receiver: Receiver<RouterProcessorMessage>,

    // Link items
    approval_codes: HashMap<String, Instant>,
    incoming_links: HashMap<String, Sender<PendingLinkControl>>,
    links: HashMap<Relation, Link>,
    
    pending_links: HashMap<Relation, (Instant, u8, Vec<Message>)>,

    // Event items
    event_subscribers: HashMap<String, HashSet<Relation>>,

    // Chord items
    chords: HashMap<String, ChordEntry>,
    chord_subscribers: HashMap<Relation, usize>,
    chord_addrs: LruCache<String, ()>,

    // Directory items
    directory_subscribers: HashSet<Relation>,
    directory: HashMap<Relation, DirectoryEntry>,

}

impl RouterProcessorState {
    pub fn new(
        config: SpiderConfig,
        state: StateData,
        sender: ProcessorSender,
        receiver: Receiver<RouterProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
            receiver,

            // Link items
            approval_codes: HashMap::new(),
            incoming_links: HashMap::new(),
            links: HashMap::new(),
            pending_links: HashMap::new(),

            // Event items
            event_subscribers: HashMap::new(),

            // Chord items
            chords: HashMap::new(),
            chord_subscribers: HashMap::new(),
            chord_addrs: LruCache::new(500),

            // Directory items
            directory_subscribers: HashSet::new(),
            directory: HashMap::new(),
        }
    }

    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            self.init().await;
            loop {
                let msg = match self.receiver.recv().await {
                    Some(msg) => msg,
                    None => break,
                };

                match msg {
                    RouterProcessorMessage::PeripheralMessage(rel, msg) => {
                        self.process_remote_message(rel, msg).await;
                    }
                    RouterProcessorMessage::NewLink(link) => {
                        self.new_link_handler(link).await;
                    }
                    RouterProcessorMessage::ApproveLink(relation) => {
                        self.approve_link_handler(relation).await;
                    }
                    RouterProcessorMessage::DenyLink(relation) => {
                        self.deny_link_handler(relation).await;
                    }
                    RouterProcessorMessage::SetApprovalCode(code) => {
                        self.set_approval_code_handler(code).await;
                    }
                    RouterProcessorMessage::ApprovedLink(link) => {
                        self.approved_link_handler(link).await;
                    }

                    RouterProcessorMessage::SendMessage(rel, msg) => {
                        self.send_msg(rel, msg).await;
                    }
                    RouterProcessorMessage::MulticastMessage(rels, msg) => {
                        self.multicast_msg(rels, msg).await;
                    }
                    // ===== Chord Operations =====
                    RouterProcessorMessage::JoinChord(addr) => {
                        self.handle_join_chord(addr).await;
                    },
                    RouterProcessorMessage::HostChord(listen_addr) => {
                        self.handle_host_chord(listen_addr).await;
                    },
                    RouterProcessorMessage::LeaveChord(name) => {
                        self.handle_leave_chord(name).await;
                    },

                    RouterProcessorMessage::AddrUpdate(id, addr) => {
                        // if there is already a link for this id, ignore. Otherwise:
                        // create a new link to this address
                        println!("Got addr update");
                        let relation = Relation{role: Role::Peer, id};
                        if !self.links.contains_key(&relation){
                            println!("Creating new link");
                            let self_relation = self.state.self_relation().await;
                            let new_link = Link::connect(self_relation, addr, relation).await;
                            if let Some(new_link) = new_link {
                                println!("New link connected");
                                self.approved_link_handler(new_link).await;
                            }else{
                                println!("Link failed to connect");
                            }
                        }
                    },

                    RouterProcessorMessage::SetName(name) => {
                        // save new name
                        let mut state_name = self.state.name().await;
                        *state_name = name.clone();
                        drop(state_name);
                        // inform listener
                        let msg = ListenProcessorMessage::SetKeyRequest(Some(name.clone()));
                        let msg = ProcessorMessage::ListenerMessage(msg);
                        self.sender.send(msg).await;
                        // update setting
                        let msg = UiProcessorMessage::SetSetting {
                            header: String::from("System"),
                            title: "Name:".into(),
                            inputs: vec![
                                ("text".to_string(), name.clone()),
                                ("textentry".to_string(), "New Name".into())
                            ],
                            cb: |idx, name, input, _|{
                                match input{
                                    spider_link::message::UiInput::Click => None,
                                    spider_link::message::UiInput::Text(name) => {
                                        let router_msg = RouterProcessorMessage::SetName(name);
                                        let msg = ProcessorMessage::RouterMessage(router_msg);
                                        Some(msg)
                                    },
                                }
                            },
                            data: String::new(),
                        };
                        self.sender.send_ui(msg).await;
                        // message name on existing channels
                        for (_, link) in &self.links{
                            let msg = RouterMessage::SetIdentityProperty("name".into(), name.clone());
                            let msg = Message::Router(msg);
                            link.send(msg).await;
                        }
                    }
                    RouterProcessorMessage::SetNickname(rel, name) => {
                        self.set_identity_system(rel, "nickname".into(), name).await;
                    }
                    RouterProcessorMessage::ClearDirectoryEntry(rel) => {
                        self.clear_directory_entry_handler(rel).await;
                    }

                    RouterProcessorMessage::Upkeep => {
                        // should check for disconnected peers, and clean them up

                        // Process pending links
                        self.process_pending_links().await;

                        // Save chord state
                        for (name, chord_entry) in self.chords.iter_mut() {
                            let associate = chord_entry.get_associate();

                            associate.send_op(AssociateRequest::GetPeerAddresses).await;
                            let peer_addrs = match associate.recv_op(None).await {
                                Some(AssociateResponse::PeerAddresses{addrs}) => {
                                    addrs
                                },
                                _ => {
                                    // chord has invalid response
                                    continue;
                                },
                            };
                            let chord_state = chord_entry.get_state_mut();
                            chord_state.add_addrs(peer_addrs.clone());

                            for peer_addr in peer_addrs {
                                self.chord_addrs.push(peer_addr, ());
                            }

                            self.state.put_chord(name, &chord_state).await;
                        }
                        // Handle chord address subscriptions
                        let mut messages = Vec::with_capacity(self.chord_subscribers.len());
                        for (rel, limit) in &self.chord_subscribers{
                            let x: Vec<String> = self.chord_addrs.iter().take(*limit).map(|(x, _)|{x.clone()}).collect();
                            let msg = Message::Router(RouterMessage::ChordAddrs(x));
                            messages.push((rel.clone(), msg));
                        }
                        for (rel, msg) in messages {
                            self.send_msg(rel, msg).await;
                        }
                        
                        // Save Directory state
                        self.state.save_directory(&self.directory).await;

                        // Clean approval codes
                        self.approval_codes.retain(|_, v|{
                            v < &mut Instant::now()
                        });
                    }
                }
            }
        });
        handle
    }

    async fn init(&mut self){
        // ===== Setup menu items =====
        // Change/Set name
        let name = self.state.name().await;
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("System"),
            title: "Name:".into(),
            inputs: vec![
                ("text".to_string(), name.clone()),
                ("textentry".to_string(), "New Name".into())
            ],
            cb: |idx, name, input, _|{
                match input{
                    spider_link::message::UiInput::Click => None,
                    spider_link::message::UiInput::Text(name) => {
                        let router_msg = RouterProcessorMessage::SetName(name);
                        let msg = ProcessorMessage::RouterMessage(router_msg);
                        Some(msg)
                    },
                }
            },
            data: String::new(),
        };
        self.sender.send_ui(msg).await;
        drop(name);

        // Connect to existing chord
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Connected Chords"),
            title: String::from("Connect:"),
            inputs: vec![("textentry".to_string(), "Chord Address".to_string())],
            cb: |idx, name, input, _|{
                match input{
                    spider_link::message::UiInput::Click => None,
                    spider_link::message::UiInput::Text(addr) => {
                        let router_msg = RouterProcessorMessage::JoinChord(addr);
                        let msg = ProcessorMessage::RouterMessage(router_msg);
                        Some(msg)
                    },
                }
            },
            data: String::new(),
        };
        self.sender.send_ui(msg).await;

        // Host new chord
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Connected Chords"),
            title: String::from("Host New:"),
            inputs: vec![("textentry".to_string(), "Chord Listen Address".to_string())],
            cb: |idx, name, input, _|{
                match input{
                    spider_link::message::UiInput::Click => None,
                    spider_link::message::UiInput::Text(addr) => {
                        let router_msg = RouterProcessorMessage::HostChord(addr);
                        let msg = ProcessorMessage::RouterMessage(router_msg);
                        Some(msg)
                    },
                }
            },
            data: String::new(),
        };
        self.sender.send_ui(msg).await;

        // Initialize chord functions
        self.init_chord_functions().await;

        // Initialize directory functions
        self.init_directory_functions().await;

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
            RouterMessage::Denied => {} // base sends this, not recv

            // Event Messages
            RouterMessage::SendEvent(name, externals, data) => {
                self.handle_send_event(rel.clone(), name, externals, data).await;
            },
            RouterMessage::Event(name, _, data) => {
                // re-route events from peers to appropriate peripherals
                // The known relation of the link is used as the from field in the event
                self.handle_event(name, rel, data).await;
            },
            RouterMessage::Subscribe(name) => {
                if rel.is_peer(){
                    return; // dont allow subscriptions from peers (at least for now)
                }
                let entry = self.event_subscribers.entry(name);
                let subscriber_set = entry.or_default();
                subscriber_set.insert(rel);
            },
            RouterMessage::Unsubscribe(name) => {
                if rel.is_peer(){
                    return; // dont allow subscriptions from peers (at least for now)
                }
                match self.event_subscribers.get_mut(&name){
                    Some(subscriber_set) => {
                        subscriber_set.remove(&rel);
                        if subscriber_set.is_empty() {
                            self.event_subscribers.remove(&name);
                        }
                    },
                    None => {}, // there were no subscribers to this message type
                }
            },

            // Directory Messages
            RouterMessage::SubscribeDir => {
                self.handle_subscribe_directory(rel).await;
            }
            RouterMessage::UnsubscribeDir => {
                self.handle_unsubscribe_directory(rel).await;
            }
            RouterMessage::AddIdentity(_) => {
                // base send this, doesnt recieve
            }
            RouterMessage::RemoveIdentity(_) => {
                // base send this, doesnt recieve
            }
            RouterMessage::SetIdentityProperty(key, value) => {
                self.set_identity_self(rel, key, value).await;
            }

            // Chord Connected Messages
            RouterMessage::SubscribeChord(limit) => {
                self.chord_subscribers.insert(rel.clone(), limit);
                let x: Vec<String> = self.chord_addrs.iter().take(limit).map(|(x, _)|{x.clone()}).collect();
                let msg = Message::Router(RouterMessage::ChordAddrs(x));
                self.send_msg(rel, msg).await;
            }
            RouterMessage::UnsubscribeChord => {
                self.chord_subscribers.remove(&rel);
            }
            RouterMessage::ChordAddrs(..) => {
                // base sends this, doesnt recieve
            }
        }
    }

    async fn approved_link_handler(&mut self, mut link: Link) {
        println!("Adding new link");
        // get reciever+relation from link
        let mut rx = match link.take_recv() {
            Some(rx) => rx,
            None => return,
        };
        let relation = link.other_relation().clone();

        // Send name
        let name = self.state.name().await;
        let msg = RouterMessage::SetIdentityProperty("name".into(), name.clone());
        drop(name);
        let msg = Message::Router(msg);
        link.send(msg).await;

        // add link relation to directory
        self.add_identity(relation.clone()).await;

        // insert pending link messages into link
        if let Some((_, _, msgs)) = self.pending_links.remove(&relation){
            for msg in msgs{
                println!("Adding message to new link");
                link.send(msg).await;
            }
        }
        
        // add link to structures
        self.links.insert(relation.clone(), link);

        // start link processor
        let channel = self.sender.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Some(msg) => {
                        match channel
                            .send(ProcessorMessage::RemoteMessage(relation.clone(), msg))
                            .await
                        {
                            Ok(_) => {}
                            Err(_) => break,
                        };
                    }
                    None => break, // connection is finished
                }
            }
        });
    }

    async fn send_msg(&mut self, relation: Relation, msg: Message) {
        // println!("Sending message: {:?}", msg);
        match self.links.get_mut(&relation) {
            Some(link) => {
                link.send(msg).await;
            }
            None => { // no link, no send at the moment (should buffer messages and start a new connection)
                //
            } 
        }
    }

    async fn multicast_msg(&mut self, relations: Vec<Relation>, msg: Message) {
        for relation in relations {
            self.send_msg(relation, msg.clone()).await
        }
    }

    async fn process_pending_link(&mut self, relation: Relation){
        if let Some((start, tries, msgs)) = self.pending_links.get_mut(&relation){
            println!("Processing pending");
            // check if pending link has connected
            if let Some(link) = self.links.get_mut(&relation){
                println!("Found link, inserting messages");
                for msg in msgs{
                    link.send(msg.clone()).await;
                }
                self.pending_links.remove(&relation);
                return;
            }

            // if number of attempts has been met, stop, remove from pending
            if *tries > 10{
                println!("Too many tries");
                self.pending_links.remove(&relation);
                return;
            }
            *tries += 1;

            // if not, test time since connection attempt
            if start.elapsed().as_secs() < 10{
                println!("Too soon to retry");
                return; // allow more time to occur
            }
            *start = Instant::now(); // reset timer

            // make connection attempt on all chords in list
            for (name, chord_entry) in self.chords.iter_mut(){
                println!("Making request on chord");
                chord_entry.resolve_id(relation.id.clone()).await;
            }
        }
    }

    async fn process_pending_links(&mut self){
        let relations: Vec<Relation> = self.pending_links.keys().cloned().collect();
        for relation in relations{
            println!("Processing pending link for relation");
            self.process_pending_link(relation).await;
        }
    }

}

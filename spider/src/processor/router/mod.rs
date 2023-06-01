use std::{collections::{HashMap, HashSet}, time::Duration};

use dht_chord::{chord::ChordHandle, associate::{AssociateRequest, AssociateResponse}, TCPChord};
use spider_link::{
    message::{Message, RouterMessage},
    Link, Relation, Role, SpiderId2048,
};
use tokio::{
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle}, time::Instant, select,
};

use crate::{config::SpiderConfig, state_data::StateData};

use super::{message::ProcessorMessage, sender::ProcessorSender, ui::UiProcessorMessage};

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

    links: HashMap<Relation, Link>,
    pending_links: HashMap<Relation, (Instant, u8, Vec<Message>)>,

    subscribers: HashMap<String, HashSet<Relation>>,

    chords: HashMap<String, (ChordHandle<String, SpiderId2048>, Sender<SpiderId2048>)>,
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

            links: HashMap::new(),
            pending_links: HashMap::new(),

            subscribers: HashMap::new(),

            chords: HashMap::new(),
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
                        self.new_link(link).await;
                    }

                    RouterProcessorMessage::SendMessage(rel, msg) => {
                        self.send_msg(rel, msg).await;
                    }
                    RouterProcessorMessage::MulticastMessage(rels, msg) => {
                        self.multicast_msg(rels, msg).await;
                    }

                    RouterProcessorMessage::JoinChord(addr) => {
                        let id = self.state.self_id().await;
                        let mut chord = TCPChord::new("0.0.0.0:1931".to_string(), id.clone());
                        let pub_addr = self.config.pub_addr.as_bytes();
                        chord.set_advert(Some(pub_addr.to_vec()));
                        let handle = chord.start(Some(addr)).await;
                        self.prepare_chord(handle).await;
                    },
                    RouterProcessorMessage::HostChord(listen_addr) => {
                        println!("Hosting chord, listening on: {:?}", listen_addr);
                        let id = self.state.self_id().await;
                        let mut chord = TCPChord::new(listen_addr, id.clone());
                        let pub_addr = self.config.pub_addr.as_bytes();
                        println!("Chord advertizing for base at: {:?}", pub_addr);
                        chord.set_advert(Some(pub_addr.to_vec()));
                        let handle = chord.start(None).await;
                        self.prepare_chord(handle).await;
                    },
                    RouterProcessorMessage::LeaveChord(name) => {
                        if let Some((handle, _)) = self.chords.remove(&name){
                           handle.stop().await;
                           // remove entry from settings page
                           let msg = UiProcessorMessage::RemoveSetting {
                                header: String::from("Connected Chords"),
                                title: name
                            };
                            self.sender.send_ui(msg).await;
                        }
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
                                self.new_link(new_link).await;
                            }else{
                                println!("Link failed to connect");
                            }
                        }
                    },

                    // add contacts list??!?
                    RouterProcessorMessage::Upkeep => {
                        // should check for disconnected peers, and clean them up
                        // for (relation, link) in &self.links{
                        //     link.
                        // }

                        // Process pending links
                        self.process_pending_links().await;
                    }
                }
            }
        });
        handle
    }

    async fn init(&mut self){
        // Connect to existing chord
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Connected Chords"),
            title: String::from("Connect:"),
            inputs: vec![("textentry".to_string(), "Chord Address".to_string())],
            cb: |idx, name, input|{
                match input{
                    spider_link::message::UiInput::Click => None,
                    spider_link::message::UiInput::Text(addr) => {
                        let router_msg = RouterProcessorMessage::JoinChord(addr);
                        let msg = ProcessorMessage::RouterMessage(router_msg);
                        Some(msg)
                    },
                }
            },
        };
        self.sender.send_ui(msg).await;

        // Host new chord
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Connected Chords"),
            title: String::from("Host New:"),
            inputs: vec![("textentry".to_string(), "Chord Listen Address".to_string())],
            cb: |idx, name, input|{
                match input{
                    spider_link::message::UiInput::Click => None,
                    spider_link::message::UiInput::Text(addr) => {
                        let router_msg = RouterProcessorMessage::HostChord(addr);
                        let msg = ProcessorMessage::RouterMessage(router_msg);
                        Some(msg)
                    },
                }
            },
        };
        self.sender.send_ui(msg).await;



    }

    async fn process_remote_message(&mut self, rel: Relation, msg: RouterMessage) {
        match msg {
            RouterMessage::Event(name, externals, data) => {
                // Send to subscribers
                let mut recipients = HashSet::new();
                if let Some(subscriber_set) = self.subscribers.get(&name){
                    for subscriber in subscriber_set{
                        // Check if source is external and dest is external, skip
                        if rel.is_peer() && subscriber.is_peer(){
                            continue;
                        }
                        if let Some(link) = self.links.get_mut(subscriber){
                            recipients.insert(subscriber.clone());
                            let router_msg = RouterMessage::Event(name.clone(), vec![], data.clone());
                            let msg = Message::Router(router_msg);
                            link.send(msg).await;
                        }
                    }
                }
                // send to externals
                for external in externals{
                    if recipients.contains(&external){
                        continue; // this recipient already recieved message via subscription
                    }
                    println!("Sending message to external...");
                    match self.links.get_mut(&external){
                        Some(link) => {
                            // send to already-connected link
                            println!("Link is connected");
                            let router_msg = RouterMessage::Event(name.clone(), vec![], data.clone());
                            let msg = Message::Router(router_msg);
                            link.send(msg).await;
                            println!("Sent");
                        },
                        None => {
                            // insert into pending links
                            println!("Link is pending");
                            match self.pending_links.get_mut(&external) {
                                Some((_, tries, pending_msgs)) => {
                                    println!("adding message to entry");
                                    let router_msg = RouterMessage::Event(name.clone(), vec![], data.clone());
                                    let msg = Message::Router(router_msg);
                                    pending_msgs.push(msg);
                                    *tries = 0;
                                },
                                None => {
                                    // not already in, need to init connection requests
                                    println!("new pending entry");
                                    let router_msg = RouterMessage::Event(name.clone(), vec![], data.clone());
                                    let msg = Message::Router(router_msg);
                                    let pending_msgs = vec![msg];
                                    let mut t = Instant::now();
                                    t = t - Duration::from_secs(600);
                                    self.pending_links.insert(external.clone(), (t, 0u8, pending_msgs));
                                    // start connection process
                                    self.process_pending_link(external).await;
                                },
                            }
                        },
                    }
                }
            },
            RouterMessage::Subscribe(name) => {
                if rel.is_peer(){
                    return; // dont allow subscriptions from peers (at least for now)
                }
                let entry = self.subscribers.entry(name);
                let subscriber_set = entry.or_default();
                subscriber_set.insert(rel);
            },
            RouterMessage::Unsubscribe(name) => {
                if rel.is_peer(){
                    return; // dont allow subscriptions from peers (at least for now)
                }
                match self.subscribers.get_mut(&name){
                    Some(subscriber_set) => {
                        subscriber_set.remove(&rel);
                        if subscriber_set.is_empty() {
                            self.subscribers.remove(&name);
                        }
                    },
                    None => {}, // there were no subscribers to this message type
                }
            },
        }
    }

    async fn new_link(&mut self, mut link: Link) {
        println!("Adding new link");
        // get reciever+relation from link
        let mut rx = match link.take_recv() {
            Some(rx) => rx,
            None => return,
        };
        let relation = link.other_relation().clone();

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
            for (name, (associate, sender)) in self.chords.iter_mut(){
                println!("Making request on chord");
                sender.send(relation.id.clone()).await;
            }
        }
    }

    async fn process_pending_links(&mut self){
        let relations: Vec<Relation> = self.pending_links.keys().cloned().collect();
        for relation in relations{
            self.process_pending_link(relation).await;
        }
    }

    async fn prepare_chord(&mut self, handle: ChordHandle<String, SpiderId2048>){
        let mut assoc = handle.get_associate().await;
        let (sender, mut receiver) = channel(50);
        let mut processor = self.sender.clone();

        // Create chord processor task
        let task_handle = tokio::spawn(async move {
            loop{
                select! {
                    request = receiver.recv() => {
                        println!("Chordprocessor recvd request");
                        match request{
                            Some(id) => {
                                let msg = AssociateRequest::GetAdvertOf{id};
                                assoc.send_op(msg).await;
                            },
                            None => {
                                println!("Chordprocessor quitting");
                                // receiver is over, quit
                                return;
                            },
                        }
                    },
                    response = assoc.recv_op(None) => {
                        println!("Chordprocessor got response");
                        match response{
                            Some(response) => {
                                if let AssociateResponse::AdvertOf { id, data } = response{
                                    println!("Got advert: {:?}", data);
                                    if let Some(data) = data{
                                        if let Ok(addr) = String::from_utf8(data){
                                            println!("Sending router update: {}", addr.to_string());
                                            let router_msg = RouterProcessorMessage::AddrUpdate(id, addr.to_string());
                                            let msg = ProcessorMessage::RouterMessage(router_msg);
                                            processor.send(msg).await;
                                        }
                                    }
                                }
                            },
                            None => {
                                println!("Chordprocessor quitting II");
                                // Chord has closed, quit
                                return;
                            },
                        }
                    }
                }
            }
        });
        
        // Create name and Settings entry
        let entry = (handle, sender);
        for i in 1.. {
            let name = format!("Chord #{i}");
            if !self.chords.contains_key(&name){
                
                // add to config list
                let msg = UiProcessorMessage::SetSetting {
                    header: String::from("Connected Chords"),
                    title: name.clone(),
                    inputs: vec![("button".to_string(), "Remove".to_string())],
                    cb: |idx, name, input|{
                        match input{
                            spider_link::message::UiInput::Click => {
                                let router_msg = RouterProcessorMessage::LeaveChord(name.to_string());
                                let msg = ProcessorMessage::RouterMessage(router_msg);
                                Some(msg)
                            },
                            spider_link::message::UiInput::Text(_) => None,
                        }
                    },
                };
                self.sender.send_ui(msg).await;
                
                self.chords.insert(name, entry);
                break;
            }
        }
    }

}

use std::{net::{SocketAddr, IpAddr}, collections::HashSet};

use dht_chord::{TCPChord, chord::ChordHandle, associate::{AssociateRequest, AssociateResponse, AssociateChannel}, adaptor::{AssociateClient, ChordAdaptor}, TCPAdaptor};
use lru::LruCache;
use spider_link::SpiderId2048;
use tokio::{sync::mpsc::{channel, Sender}, select};

use crate::processor::{router::RouterProcessorMessage, message::ProcessorMessage, ui::UiProcessorMessage, sender::ProcessorSender};

use super::RouterProcessorState;




// Chord processing functions
impl RouterProcessorState {

    pub(crate) async fn init_chord_functions(&mut self){
        // read from state and connect to each chord
        for chord_name in self.state.chord_names().await {
            if let Some(state) = self.state.get_chord(&chord_name).await{
                let processor_sender = self.sender.clone();
                let id = self.state.self_id().await;
                let join_or_host = true;

                if let Some(chord_entry) = ChordEntry::start_chord(processor_sender, id, state, join_or_host).await{
                    let name = self.get_next_name();
                    self.install_chord(name, chord_entry).await;
                }

            }else{
                println!("Could not start saved chord: {chord_name}");
            }
        }
    }

    pub(crate) async fn handle_join_chord(&mut self, addr: String) {

        // listen addr needs to get next available port number
        let listen_port: u16 = match self.get_next_port(){
            Some(port) => port,
            None => return, // No more available ports
        };
        let listen_addr = format!("0.0.0.0:{}", listen_port);

        // get pub addr from new peer, but use port from base listen addr
        let mut ac = TCPAdaptor::<String, SpiderId2048>::associate_client(addr.clone());
        let (pub_addr, advert_addr) = match ac.public_address().await{
            Some(addr) => {
                let base_listen: SocketAddr = self.config.listen_addr.parse().expect("Base must be listening");
                match addr.parse::<SocketAddr>(){
                    Ok(mut addr) => {
                        addr.set_port(base_listen.port());
                        let advert_addr = addr.to_string();
                        addr.set_port(listen_port);
                        let pub_addr = addr.to_string();
                        (pub_addr, advert_addr)
                    },
                    Err(_) => return, // cant get public addr
                }
            },
            None => return, // cant get public addr
        };

        // make chord state
        let mut state = ChordState::new(listen_addr, pub_addr, advert_addr);
        // New chord, only join addr is whats given
        state.add_addrs(vec![addr]);

        // Gather other parameters
        let id = self.state.self_id().await;
        let processor_sender = self.sender.clone();
        let join_or_host = false;

        if let Some(chord_entry) = ChordEntry::start_chord(processor_sender, id, state, join_or_host).await{
            let name = self.get_next_name();
            self.install_chord(name, chord_entry).await;
        }
    }

    pub(crate) async fn handle_host_chord(&mut self, advert_addr: String) {
        
        // listen addr needs to get next available port number
        let listen_port: u16 = match self.get_next_port(){
            Some(port) => port,
            None => return, // No more available ports
        };

        // pub addr should be from input, but filling in the port from the listen address if missing
        let (pub_addr, advert_addr) = match advert_addr.parse::<SocketAddr>(){
            Ok(advert_addr) => {
                let mut pub_addr = advert_addr.clone();
                pub_addr.set_port(listen_port);
                (pub_addr.to_string(), advert_addr.to_string())
            },
            Err(_) => {
                // could not parse as socket addr, try as ip addr
                match advert_addr.parse::<IpAddr>(){
                    Ok(advert_addr) => {
                        let spider_listen: SocketAddr = self.config.listen_addr.parse().unwrap();
                        // add port from spider listen addr
                        let pub_addr: SocketAddr = (advert_addr, listen_port).into();
                        let advert_addr: SocketAddr = (advert_addr, spider_listen.port()).into();
                        (pub_addr.to_string(), advert_addr.to_string())
                    },
                    Err(_) => {
                        return; // cant create address
                    },
                }
            },
        };        

        let id = self.state.self_id().await;
        
        let processor_sender = self.sender.clone();
        let listen_addr = format!("0.0.0.0:{}", listen_port);
        let join_addrs = vec![];
        let join_or_host = true;

        let mut state = ChordState::new(listen_addr, pub_addr, advert_addr);
        state.add_addrs(join_addrs);

        if let Some(chord_entry) = ChordEntry::start_chord(processor_sender, id, state, join_or_host).await{
            let name = self.get_next_name();
            self.install_chord(name, chord_entry).await;
        }
    }

    pub(crate) async fn handle_leave_chord(&mut self, name: String){
        if let Some(chord_entry) = self.chords.remove(&name){
            // Stop Chord
            chord_entry.stop_chord().await;
            // Remove from state
            self.state.remove_chord(&name).await;
            // remove entry from settings page
            let msg = UiProcessorMessage::RemoveSetting {
                header: String::from("Connected Chords"),
                title: name
            };
            self.sender.send_ui(msg).await;
        }
    }
}

// Chord helper functions
impl RouterProcessorState {

    fn get_next_name(&self) -> String{
        for i in 1.. {
            let name = format!("Chord #{i}");
            if !self.chords.contains_key(&name){
                return name;
            }
        }
        return String::from("default name");
    }

    fn get_next_port(&self) -> Option<u16> {
        // search through connected chords to find the next available port
        // starting at a configurable base port
        let mut choices: HashSet<u16> = (1932..1950).collect();
        for (_, entry) in &self.chords{
            match entry.state.listen_addr.parse::<SocketAddr>(){
                Ok(addr) => {
                    let port = addr.port();
                    choices.remove(&port);
                },
                Err(_) => continue,
            }
        }

        choices.iter().next().copied()
    }

    async fn install_chord(&mut self, name: String, chord_entry: ChordEntry){
        // Prepare settings entry
        let listen_addr = chord_entry.state.listen_addr.clone();
        let pub_addr = chord_entry.state.pub_addr.clone();
        let status = format!("Listen Addr: {:?} | Pub Addr: {:?}", listen_addr, pub_addr);
        let msg = UiProcessorMessage::SetSetting {
            header: String::from("Connected Chords"),
            title: name.clone(),
            inputs: vec![
                ("text".to_string(), status),
                ("button".to_string(), "Remove".to_string()),
            ],
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


        // insert into chords list
        self.chords.insert(name, chord_entry);
    }
}



pub struct ChordEntry{
    handle: ChordHandle<String, SpiderId2048>,
    associate: AssociateChannel<String, SpiderId2048>,
    addr_sender: Sender<SpiderId2048>,

    state: ChordState,
}

impl ChordEntry{


    pub fn get_state_mut(&mut self) -> &mut ChordState{
        &mut self.state
    }

    pub fn get_associate(&mut self) -> &mut AssociateChannel<String, SpiderId2048>{
        &mut self.associate
    }

    async fn start_chord(
        processor_sender: ProcessorSender,
        id: SpiderId2048,
        state: ChordState,
        join_or_host: bool,
    ) -> Option<Self> {
        let listen_addr = state.listen_addr.clone();
        let pub_addr = state.pub_addr.clone();
        let advert_addr = state.advert_addr.clone();
        let join_addrs: Vec<String> = state.get_addrs().map(|x|{x.0.clone()}).collect();

        let mut chord = TCPChord::new(listen_addr.clone(), id.clone());
        chord.set_listen_addr(listen_addr);
        chord.set_self_addr(pub_addr);
        chord.set_advert(Some(advert_addr.as_bytes().to_vec()));
        if join_addrs.len() != 0{
            chord.set_join_list(join_addrs.clone());
        }
        chord.set_join_or_host(join_or_host);
        

        match chord.start(None).await {
            Some(handle) => {
                // get associate
                let associate = handle.get_associate().await;

                // start addr_sender
                let sender_associate = handle.get_associate().await;
                let addr_sender = Self::create_addr_sender(sender_associate, processor_sender);

                // return chord
                Some(Self{
                    handle,
                    associate,
                    addr_sender,
                    state,
                })
            },
            None => None,
        }
    }
    
    fn create_addr_sender(mut associate: AssociateChannel<String, SpiderId2048>, processor_sender: ProcessorSender) -> Sender<SpiderId2048>{
        // Create chord processor task
        let (sender, mut receiver) = channel(50);
        let task_handle = tokio::spawn(async move {
            loop{
                select! {
                    request = receiver.recv() => {
                        println!("Chordprocessor recvd request");
                        match request{
                            Some(id) => {
                                let msg = AssociateRequest::GetAdvertOf{id};
                                associate.send_op(msg).await;
                            },
                            None => {
                                println!("Chordprocessor quitting");
                                // receiver is over, quit
                                return;
                            },
                        }
                    },
                    response = associate.recv_op(None) => {
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
                                            processor_sender.send(msg).await;
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
        sender
    }

    pub async fn stop_chord(self){
        self.handle.stop().await;
    }

    pub async fn resolve_id(&mut self, id: SpiderId2048){
        self.addr_sender.send(id).await;
    }

}


pub struct ChordState{
    pub listen_addr: String,
    pub pub_addr: String,
    pub advert_addr: String,
    join_addrs: LruCache<String, ()>,
}

impl ChordState{
    pub fn new(listen_addr: String, pub_addr: String, advert_addr: String) -> Self{
        Self{
            listen_addr,
            pub_addr,
            advert_addr,
            join_addrs: LruCache::new(500),
        }
    }
    pub fn add_addrs(&mut self, addrs: Vec<String>){
        for addr in addrs.iter().rev(){
            self.join_addrs.push(addr.clone(), ());
        }
    }
    pub fn get_addrs(&self) -> lru::Iter<'_, String, ()>{
        self.join_addrs.iter()
    }
    pub fn clear_addrs(&mut self){
        self.join_addrs.clear();
    }
}

// use dht_chord::{TCPAdaptor, adaptor::{ChordAdaptor, AssociateClient}};
use tracing::info;

use std::path::{Path, PathBuf};


use spider_link::{Link, message::Message, Relation};
use crate::{SpiderClientState, state::AddressStrategy};


pub struct SpiderClient{
    state_path: Option<PathBuf>,
    state: SpiderClientState,
    link: Option<Link>,
}


impl SpiderClient {

    pub fn new() -> Self {
        SpiderClient{
            state_path: None,
            state: SpiderClientState::new(),
            link: None,
        }
    }

    pub fn from_file(path: &Path) -> Self {
        let state = SpiderClientState::from_file(path);
        Self {
            state_path: Some(path.to_path_buf()),
            state,
            link: None,
        }
    }

    pub fn save(&self){
        if let Some(path) = &self.state_path{
            self.state.to_file(path)
        }
    }

    pub fn self_relation(&self) -> Relation{
        self.state.self_relation.relation.clone()
    }

    pub fn has_host_relation(&self) -> bool {
        self.state.host_relation.is_some()
    }

    pub fn set_host_relation(&mut self, relation: Relation) {
        self.state.host_relation = Some(relation);
    }

    pub fn is_connected(&self) -> bool {
        self.link.is_some()
    }

    pub fn add_strat(&mut self, strat: AddressStrategy) {
        self.state.strategies.push(strat);
    }

    pub fn set_state_path(&mut self, path: &PathBuf){
        self.state_path = Some(path.to_path_buf())
    }

    pub async fn connect(&mut self) {
        if self.is_connected(){
            return;
        }

        // find address
        for strat in self.state.strategies.clone(){
            let address = self.find_address(&strat).await.unwrap();
            // connect to address
            self.connect_to(address).await;
            if self.is_connected(){
                break;
            }
        }
        
    }

    async fn find_address(&self, strat: &AddressStrategy) -> Option<String> {
        match strat{
            AddressStrategy::Localhost => Some(String::from("localhost")),
            AddressStrategy::LastAddr => self.state.last_addr.clone(),
            AddressStrategy::Addr(addr) => Some(addr.clone()),
            AddressStrategy::Beacon => todo!(),
            AddressStrategy::Chord => {
                // check chord for address
                // let address_list = self.state.addr_list.as_ref();
                // if let Some(addr_list) = address_list {
                //     for addr in addr_list {
                //         let mut associate: AssociateClient<String, u32> = TCPAdaptor::associate_client(addr.clone());
                //         if let Some((id, addr)) = associate.successor_of(self.state.node_id).await{
                //             if id == self.state.node_id {
                //                 return Some(addr);
                //             }
                //         }
                //     }
                // }
                None
            },
        }
    }

    pub async fn connect_to(&mut self, addr: String) -> bool{
        if self.link.is_some(){
            return true;
        }
        let host_relation = if let Some(relation) = self.state.host_relation.clone() {
            relation
        }else{
            return false;
        };

        info!("Connecting to address: {}", addr);
        // connect to address
        let own_relation = self.state.self_relation.clone();
        let link = spider_link::Link::connect(own_relation, addr, host_relation).await;
        // start connection handler task
        
        self.link = link;
        return true;
    }

    async fn _send(&mut self, msg: &Message) -> bool{
        match self.link{
            Some(ref link) => {
                link.send(msg.clone()).await;
                true
            },
            None => {
                false
            },
        }
    }

    pub async fn send(&mut self, msg: Message){
        loop {
            let res = self._send(&msg).await;
            if res || !self.state.auto_reconnect {
                break;
            }else{
                self.connect().await;
            }
        }
    }

    async fn _recv(&mut self) -> Option<Message>{
        match self.link{
            Some(ref mut link) => {
                link.recv().await
            },
            None => None,
        }
    }

    pub async fn recv(&mut self) -> Option<Message> {
        loop {
            let res = self._recv().await;

            if res.is_none() && self.state.auto_reconnect {
                self.connect().await;
            }else{
                break res;
            }
        }
    }

}




use std::{
    fs, io,
    path::{Path, PathBuf}, net::SocketAddr,
};

use crate::SpiderClientState;
use spider_link::Relation;

use self::processor::SpiderClientProcessor;

pub mod processor;
mod channel;
pub use channel::ClientChannel;

mod message;
pub use message::{ClientControl, ClientResponse};

#[derive(Debug, Clone)]
pub struct SpiderClientBuilder {
    state_path: Option<PathBuf>,
    state: SpiderClientState,
}

impl SpiderClientBuilder {
    pub fn new() -> Self {
        SpiderClientBuilder {
            state_path: None,
            state: SpiderClientState::new(),
        }
    }

    pub fn load(path: &Path) -> Self {
        let state = SpiderClientState::from_file(path);
        Self {
            state_path: Some(path.to_path_buf()),
            state,
        }
    }

    pub fn load_or_set<F: FnOnce(&mut SpiderClientBuilder)>(path: &Path, func: F) -> Self {
        match fs::read_to_string(&path).and_then(|data| {
            SpiderClientState::from_string(data).ok_or(io::ErrorKind::InvalidData.into())
        }) {
            Ok(state) => Self {
                state_path: Some(path.to_owned()),
                state,
            },
            Err(_) => {
                let mut client = Self {
                    state_path: Some(path.to_owned()),
                    state: SpiderClientState::new(),
                };
                func(&mut client);
                client.save();
                client
            },
        }
    }

    pub fn save(&self) {
        if let Some(path) = &self.state_path {
            self.state.to_file(path)
        }
    }

    pub fn start(self, enable_recv: bool) -> ClientChannel{
        let (channel, _) = SpiderClientProcessor::start(self.state_path, self.state, enable_recv);
        channel
    }

    // State modification functions
    pub fn set_state_path(&mut self, path: &PathBuf) {
        self.state_path = Some(path.to_path_buf())
    }

    pub fn self_relation(&self) -> &Relation {
        &self.state.self_relation.relation
    }

    pub fn has_host_relation(&self) -> bool {
        self.state.host_relation.is_some()
    }

    pub fn set_host_relation(&mut self, relation: Relation) {
        self.state.host_relation = Some(relation);
    }
    pub fn clear_host_relation(&mut self) {
        self.state.host_relation = None;
    }

    // Connection strategies
    pub fn enable_last_addr(&mut self, set: bool){
        self.state.last_addr_enable = set;
    }
    pub fn set_last_addr(&mut self, set: Option<String>){
        self.state.set_last_addr(set);
    }
    
    // Beacon
    pub fn enable_beacon(&mut self, set: bool){
        self.state.beacon_enable = set;
    }

    // Chord
    pub fn enable_chord(&mut self, set: bool){
        self.state.chord_enable = set;
    }
    pub fn set_chord_addrs(&mut self, set: Vec<String>){
        self.state.chord_addrs = set;
    }

    // Fixed addresses
    pub fn enable_fixed_addrs(&mut self, set: bool){
        self.state.fixed_addr_enable = set;
    }
    pub fn set_fixed_addrs(&mut self, addrs: Vec<String>){
        self.state.fixed_addrs = addrs;
    }
}

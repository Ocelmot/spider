use std::collections::HashMap;

use serde::{Serialize, Deserialize};

use crate::Relation;

use super::DatasetData;



#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterMessage {
    // Authorization messages
    Pending,
    ApprovalCode(String),
    Approved,
    Denied,

    // Event messages
    SendEvent(String, Vec<Relation>, DatasetData),
    Event(String, Relation, DatasetData),
    Subscribe(String),
    Unsubscribe(String),

    // Directory messages
    SubscribeDir,
    UnsubscribeDir,
    AddIdentity(DirectoryEntry),
    RemoveIdentity(Relation),
    SetIdentityProperty(String, String),

    // Chord messages
    SubscribeChord(usize),
    UnsubscribeChord,
    ChordAddrs(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry{
    relation: Relation,
    properties: HashMap<String, String>,
}

impl DirectoryEntry{
    pub fn new(rel: Relation) -> Self{
        Self{
            relation: rel,
            properties: HashMap::new(),
        }
    }

    pub fn relation(&self)-> &Relation {
        &self.relation
    }

    pub fn get(&self, key: &str)-> Option<&String> {
        self.properties.get(key)
    }
    pub fn set(&mut self, key: String, value: String){
        self.properties.insert(key, value);
    }
}

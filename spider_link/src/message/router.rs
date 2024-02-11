use std::collections::HashMap;

use serde::{Serialize, Deserialize};

use crate::Relation;

use super::DatasetData;


/// RouterMessage manages the relationship between the two members of the Spider
/// network. There are four general categories of messages of this type:
/// Authorization, Event, Directory, and Chord.
/// Authorization messages negotiate whether the base will allow the connection.
/// Event messages control how messages with arbitrary data are sent through
/// the network.
/// Directory messages allow one member of the network to tell another member
/// its nickname or get a list of nicknames known by the base.
/// (Like a contact list)
/// Chord messages allow peripherals to get a list of addresses in the base's
/// chord in order for those peripherals to be able to use the chord to find
/// the base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterMessage {
    // Authorization messages
    /// Indicates that this connection is in the pending state, and will not
    /// process any messages until approved.
    Pending,
    /// Allows a connecting member of the network to approve themselves if they
    /// have a valid code. Codes are communicated via some other mechanism.
    ApprovalCode(String),
    /// The connection is now approved, any messages sent before this point will
    /// now be processed.
    Approved,
    /// The connection has been denied, the connection will be
    /// closed after this.
    Denied,

    // Event messages
    /// Send a message with a type, a set of recipients, and some data.
    SendEvent(String, Vec<Relation>, DatasetData),
    /// A received event with a type, sender, and some data.
    Event(String, Relation, DatasetData),
    /// Request to receive messages of a particular type that are being routed
    /// by the base.
    Subscribe(String),
    /// Stop receiving messages of a particular type routed by the base.
    Unsubscribe(String),

    // Directory messages
    /// Request to receive notifications of changes to the directory.
    SubscribeDir,
    /// Request to stop receiving notifications of changes to the directory.
    UnsubscribeDir,
    /// An entry in the directory has changed, this is the new entry.
    AddIdentity(DirectoryEntry),
    /// An entry in the directory has been removed,
    /// this is the removed relation.
    RemoveIdentity(Relation),
    /// Indicate to the other member of this connection to update this member's
    /// identity properties.
    SetIdentityProperty(String, String),

    // Chord messages
    /// Request to receive the n most recent addresses in the base's chord in
    /// order to allow peripherals to use the chord to connect to lookup the
    /// base's address.
    SubscribeChord(usize),
    /// Request to stop receiving recent chord addresses from the base.
    UnsubscribeChord,
    /// The n most recent chord addresses.
    ChordAddrs(Vec<String>),
}

/// A DirectoryEntry holds details about some other member of the
/// spider network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry{
    relation: Relation,
    properties: HashMap<String, String>,
}

impl DirectoryEntry{
    /// Create a new, empty DirectoryEntry for the provided [Relation].
    pub fn new(rel: Relation) -> Self{
        Self{
            relation: rel,
            properties: HashMap::new(),
        }
    }

    /// Get the [Relation] this DirectoryEntry describes.
    pub fn relation(&self)-> &Relation {
        &self.relation
    }

    /// Get the value of one of the properties in this DirectoryEntry.
    pub fn get(&self, key: &str)-> Option<&String> {
        self.properties.get(key)
    }

    /// Set the value of one of the properties in this DirectoryEntry.
    pub fn set(&mut self, key: String, value: String){
        self.properties.insert(key, value);
    }
}

use base64::{engine::general_purpose, Engine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{Relation, SelfRelation};

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

    /// A list of addresses that could be used to connect to this node
    Addrs(Vec<String>),

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

    // Invitation messages
    /// An invite that can be sent to another node, allowing them to connect.
    Invite(Invite),

    /// Request that an invite of the indicated type be sent to the peripheral.
    /// This is so that it can be exchanged to another node to connect.
    GenerateInvite,
}

/// A DirectoryEntry holds details about some other member of the
/// spider network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    relation: Relation,
    properties: HashMap<String, String>,
}

impl DirectoryEntry {
    /// Create a new, empty DirectoryEntry for the provided [Relation].
    pub fn new(rel: Relation) -> Self {
        Self {
            relation: rel,
            properties: HashMap::new(),
        }
    }

    /// Get the [Relation] this DirectoryEntry describes.
    pub fn relation(&self) -> &Relation {
        &self.relation
    }

    /// Get the value of one of the properties in this DirectoryEntry.
    pub fn get(&self, key: &str) -> Option<&String> {
        self.properties.get(key)
    }

    /// Set the value of one of the properties in this DirectoryEntry.
    pub fn set(&mut self, key: String, value: String) {
        self.properties.insert(key, value);
    }
}

/// An invite to establish a connection to some other base node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    rel: Relation,
    addrs: Vec<String>,
    invite_code: String,
}

impl Invite {
    /// Create a new Invite to establish a connection to this node.
    pub fn new(self_rel: &SelfRelation, addrs: Vec<String>, invite_code: String) -> Self {
        Self {
            rel: self_rel.relation.clone(),
            addrs,
            invite_code,
        }
    }

    /// Get a reference to this Invite's Relation
    pub fn rel(&self) -> &Relation {
        &self.rel
    }

    /// Get a reference to this Invite's list of addresses
    pub fn addrs(&self) -> &Vec<String> {
        &self.addrs
    }

    /// Get a reference to this Invite's invite code
    pub fn invite_code(&self) -> &String {
        &self.invite_code
    }

    /// Convert this invite into a string representation to send to another node.
    pub fn to_base64(&self) -> String {
        let bytes = serde_cbor::to_vec(self).unwrap();
        general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Convert this invite from a string representation sent from another node.
    pub fn from_base64<T>(s: T) -> Option<Self>
    where
        T: AsRef<[u8]>,
    {
        let bytes = general_purpose::URL_SAFE_NO_PAD.decode(s).ok()?;
        serde_cbor::from_slice(&bytes).ok()
    }

    /// Generate the sha 256 hash of this invite
    pub fn sha256(&self) -> String {
        let bytes = serde_cbor::to_vec(self).unwrap();
        sha256::digest(bytes.as_slice())
    }
}

#[cfg(test)]
mod tests {

    // use rand::random;
    // use tracing::info;
    // use tracing_test::traced_test;
    // use veilid_core::{CryptoKey, FourCC, TypedKey};

    // use crate::SelfRelation;

    // use super::*;

    // Test the invite's roundtrip with other means
    // #[test]
    // #[traced_test]
    // fn invite_round_trip() {
    //     let rel = SelfRelation::debug_get(5).relation;
    //     let key = CryptoKey::new(random());
    //     let typed_key = TypedKey::new(FourCC::default(), key);
    //     let addrs = vec![typed_key.to_string()];
    //     let invite_code = String::from("test invite code");
    //     let invite = Invite {
    //         rel,
    //         addrs,
    //         invite_code,
    //     };

    //     let serialized = invite.to_base64();
    //     info!("serialized: {}", serialized);
    //     let deserialized = Invite::from_base64(serialized).expect("deserialization should succeed");

    //     assert_eq!(invite.rel, deserialized.rel);
    //     assert_eq!(invite.addrs, deserialized.addrs);
    //     assert_eq!(invite.invite_code, deserialized.invite_code);
    // }
}

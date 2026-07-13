use link_set::links::Address;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::{
    LinkResult, Relation, SelfRelation, error::{ErrorKind, ProblemWrap}, utils::base62::{base62_decode, base62_encode},
};

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
    Addrs(Vec<Address>),

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
    ///
    /// When sent from the base to a peripheral, this is a newly generated
    /// invite to be sent. When sent to the base, this is an Invite from another
    /// user and will be used to try to connect to them.
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
    #[serde(default)]
    addrs: HashSet<Address>,
    properties: HashMap<String, String>,
}

impl DirectoryEntry {
    /// Create a new, empty DirectoryEntry for the provided [Relation].
    pub fn new(rel: Relation) -> Self {
        Self {
            relation: rel,
            addrs: HashSet::new(),
            properties: HashMap::new(),
        }
    }

    /// Get the [Relation] this DirectoryEntry describes.
    pub fn relation(&self) -> &Relation {
        &self.relation
    }

    /// A reference to the addrs property of this entry.
    pub fn addrs(&self) -> &HashSet<Address> {
        &self.addrs
    }

    /// A mutable reference to the addrs property of this entry.
    pub fn addrs_mut(&mut self) -> &mut HashSet<Address> {
        &mut self.addrs
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

const INVITE_V1: &'static [u8; 8] = b"SPDRIV01";

/// An invite to establish a connection to some other base node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    rel: Relation,
    addrs: Vec<Address>,
    invite_code: Vec<u8>,
}

impl Invite {
    /// Create a new Invite to establish a connection to this node.
    pub fn new(self_rel: &SelfRelation, addrs: Vec<Address>, invite_code: Vec<u8>) -> Self {
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
    pub fn addrs(&self) -> &Vec<Address> {
        &self.addrs
    }

    /// Get a reference to this Invite's invite code
    pub fn invite_code_string(&self) -> String {
        base62_encode(&self.invite_code)
    }

    /// Convert this invite into a string representation to send to another node.
    pub fn encode(&self) -> String {
        let mut bytes = Vec::new();
        
        // rel
        bytes.extend_from_slice(&self.rel.to_minimal_bytes());
        // addrs
        let addr_count = self.addrs.len() as u8;
        bytes.push(addr_count);
        for addr in &self.addrs {
            let addr_bytes = addr.to_bytes();
            let addr_len = addr_bytes.len() as u16;
            bytes.extend_from_slice(&addr_len.to_be_bytes());
            bytes.extend_from_slice(&addr_bytes);
        }
        // invite code
        bytes.extend_from_slice(&self.invite_code);
        
        let encoded = base62_encode(&bytes);
        [str::from_utf8(INVITE_V1).unwrap(), &encoded].concat()
    }

    /// Convert this invite from a string representation sent from another node.
    pub fn decode<T>(s: T) -> LinkResult<Self>
    where
        T: AsRef<str>,
    {
        let s = s.as_ref().replace(&['`', ' ', '\t', '\r', '\n'][..], "");
        let (invite_ver, remainder) = s.as_bytes().split_first_chunk().wrap_problem(ErrorKind::Deserialization)?;
        let bytes = base62_decode(remainder)?;
        let remainder = bytes.as_slice();

        // check invite version
        match invite_ver {
            INVITE_V1 => {
                let (rel_bytes, remainder) = remainder.split_at_checked(257).wrap_problem(ErrorKind::Deserialization)?;
                let rel = Relation::from_minimal_bytes(rel_bytes)?;

                let (addr_count_bytes, mut remainder) = remainder.split_at_checked(1).wrap_problem(ErrorKind::Deserialization)?;
                let addr_count = addr_count_bytes[0];
                let mut addrs = Vec::with_capacity(addr_count.into());
                for _ in 0..addr_count {
                    let loop_remainder = remainder;
                    let (addr_len_bytes, loop_remainder) = loop_remainder.split_at_checked(2).wrap_problem(ErrorKind::Deserialization)?;
                    let addr_len = u16::from_be_bytes(addr_len_bytes.try_into().unwrap());
                    let (addr_bytes, loop_remainder) = loop_remainder.split_at_checked(addr_len.into()).wrap_problem(ErrorKind::Deserialization)?;
                    let addr = Address::from_bytes(addr_bytes).wrap_problem(ErrorKind::Deserialization)?;
                    addrs.push(addr);
                    remainder = loop_remainder;
                }

                let invite_code = remainder.to_vec();

                Ok(Self { rel, addrs, invite_code })
            },
            _ => {
                return Err(ErrorKind::Deserialization)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use tracing::info;
    use tracing_test::traced_test;

    use crate::SelfRelation;

    use super::*;

    // Test the invite's roundtrip with other means
    #[test]
    #[traced_test]
    fn invite_round_trip() {
        let rel = SelfRelation::debug_get(5).relation;
        let addr = Address::new("test", "test_addr");
        let addrs = vec![addr];
        let invite_code = b"test invite code".to_vec();
        let invite = Invite {
            rel,
            addrs,
            invite_code,
        };

        let serialized = invite.encode();
        info!("serialized: {}", serialized);
        let deserialized = Invite::decode(serialized).expect("deserialization should succeed");

        assert_eq!(invite.rel, deserialized.rel);
        assert_eq!(invite.addrs, deserialized.addrs);
        assert_eq!(invite.invite_code, deserialized.invite_code);
    }
}

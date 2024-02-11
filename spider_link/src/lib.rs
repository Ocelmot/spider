#![deny(missing_docs)]


//! The spider_link crate encapsulates everything realated to esablishing
//! a link between any two members of the spider network.
//! 
//! To develop a peripheral for the spider network, use the spider_client
//! crate. That crate includes this crate, re-exports the needed types and
//! functions, and adds useful functionality around finding and
//! reestablishing connections.
//! 
//! There are two broad categories of connection: the Peer, and the Peripheral.
//! A Peer connection represents a link from one base to another base.
//! The primary use for this is to send data, as much of the other
//! functionality of the base is prohibited to other peers.
//! A Peripheral connection represents a connection to a process that is
//! closely associated with the base. This could be an embedded device or
//! a mobile app used to interface with the base. These types of
//! connections are trusted. 


use std::sync::Arc;

use base64::{engine::general_purpose, Engine};
use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
    RsaPrivateKey,
};
use serde::{Deserialize, Serialize};
use tokio::{net::ToSocketAddrs, sync::{mpsc::Receiver, Mutex}};

pub mod link;
pub use link::Link;
pub mod message;
pub mod id;
use id::SpiderId;
pub mod beacon;
mod keyfile;
pub use keyfile::Keyfile;

// TODO: This should be renamed to SpiderId, and the generic id
// renamed to something else.
/// The id used by the spider protocol.
/// The id is a 2048 bit public key, it requires 294 bytes to represent.
pub type SpiderId2048 = SpiderId<294>;

/// The type of relationship of one member of the link.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Role {
    /// This member of the link is a peripheral, it can use any of the
    /// services the base provides.
    Peripheral,
    /// This member of the link is a base, if it is connected to a peripheral
    /// it can manage that peripheral. If it is connected to another base,
    /// it can pass data messages to that base for further processing.
    Peer,
}

/// The Relation includes both the id and role of a member of the network.
/// Typically represents the other side of the connection.
/// The local side of the connection is typically a [SelfRelation].
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
pub struct Relation {
    /// The role in the connection that this member fills.
    pub role: Role,
    /// The id of the newwork member
    pub id: SpiderId2048,
}

impl Relation{
    /// Returns true of this relation represents a peripheral
    pub fn is_peripheral(&self) -> bool{
        if let Role::Peripheral = self.role{
            true
        }else{
            false
        }
    }

    /// Returns true if this relation represents a peer
    pub fn is_peer(&self) -> bool{
        if let Role::Peer = self.role{
            true
        }else{
            false
        }
    }

    /// Returns a base 64 encoded representation of this relation
    pub fn to_base64(&self) -> String {
        let role: u8 = match self.role {
            Role::Peripheral => 1,
            Role::Peer => 0,
        };
        let mut bytes = self.id.clone().to_bytes().to_vec();
        bytes.push(role);
        general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Optionally returns a Relation from a decoded base64 string
    pub fn from_base64(s: String) -> Option<Self> {
        match general_purpose::URL_SAFE_NO_PAD.decode(s){
            Ok(mut v) => {
                let role = match v.pop()? {
                    0 => Role::Peer,
                    1 => Role::Peripheral,
                    _ => return None,
                };
                let bytes= match v.try_into() {
                    Ok(bytes) => bytes,
                    Err(_) => return None,
                };
                let id = SpiderId2048::from_bytes(bytes);
                Some(Self {
                    role,
                    id
                })
            },
            Err(_) => None,
        }
    }

    /// Optionally returns a relation from an id from a base64 encoded
    /// string, and a role of peripheral.
    pub fn peripheral_from_base_64<S: Into<String>>(s: S) -> Option<Self>{
        match SpiderId2048::from_base64(s) {
            Some(id) => {
                Some(Self {
                    role: Role::Peripheral,
                    id
                })
            },
            None => None,
        }
    }

    /// Optionally returns a relation from an id from a base64 encoded
    /// string, and a role of peer.
    pub fn peer_from_base_64<S: Into<String>>(s: S) -> Option<Self>{
        match SpiderId2048::from_base64(s) {
            Some(id) => {
                Some(Self {
                    role: Role::Peer,
                    id
                })
            },
            None => None,
        }
    }

    /// Returns a string with the sha256 hash of the the relation
    pub fn sha256(&self) -> String{
        let role: u8 = match self.role {
            Role::Peripheral => 1,
            Role::Peer => 0,
        };
        let mut bytes = self.id.clone().to_bytes().to_vec();
        bytes.push(role);
        sha256::digest(bytes.as_slice())
    }
}

/// A self relation functions similarly to a [Relation], but it also includes
/// the private key that corresponds to the id.
/// Typically represents the local side of a connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfRelation {
    /// The private key of the local node
    pub priv_key_der: Vec<u8>,
    /// The Relation of the local node
    pub relation: Relation,
}

impl SelfRelation {
    /// Create a SelfRelation from a private key and a role
    pub fn from_key(key: RsaPrivateKey, role: Role) -> Self {
        let priv_bytes = key.to_pkcs8_der().unwrap().as_ref().to_vec();
        let pub_bytes = key.to_public_key().to_public_key_der().unwrap();
        let id = SpiderId::from_bytes(pub_bytes.as_ref().try_into().unwrap());
        Self {
            priv_key_der: priv_bytes,
            relation: Relation { id, role },
        }
    }
    /// Create a SelfRelation from a der representation
    /// of a private key and a role
    pub fn from_der(bytes: &[u8], role: Role) -> Self {
        let key = RsaPrivateKey::from_pkcs8_der(bytes).unwrap();
        Self::from_key(key, role)
    }

    /// Generate a new SelfRelation with the given Role.
    pub fn generate_key(role: Role) -> Self {
        let mut rng = rand::thread_rng();
        let key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
        Self::from_key(key, role)
    }

    /// Get the private key of this SelfRelation
    pub fn private_key(&self) -> RsaPrivateKey {
        RsaPrivateKey::from_pkcs8_der(&self.priv_key_der).unwrap()
    }

    /// Optionally establish a link to an ip address using this
    /// SelfRelation and a provided other Relation
    pub async fn connect_to<A: ToSocketAddrs>(&self, addr: A, relation: Relation) -> Option<Link> {
        Link::connect(self.clone(), addr, relation).await
    }

    /// Start a listener using this SelfRelation and an ip address to bind
    /// the listener to. Returns both a channel through which new [Links](Link)
    /// will be sent, and a mutex to control if this listener will respond
    /// to key requests.
    pub fn listen<A: ToSocketAddrs + Send + 'static>(&self, addr: A) -> (Receiver<Link>, Arc<Mutex<Option<String>>>) {
        Link::listen(self.clone(), addr)
    }
}


impl Eq for Relation{}
impl PartialOrd for Relation{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.role.partial_cmp(&other.role) {
            Some(core::cmp::Ordering::Equal) => {}
            ord => return ord,
        }
        self.id.partial_cmp(&other.id)
    }
}
impl Ord for Relation{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap() // There is no None option in partial cmp
    }
}
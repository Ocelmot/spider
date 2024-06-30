use std::collections::HashMap;

use base64::{engine::general_purpose, Engine};
use serde::{Serialize, Deserialize};
use veilid_core::{PublicKey, TypedKey};

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

    /// First sent by a connected entity to indicate that Veilid is available
    /// The Base should store this key and indicate that future connections
    /// should use it. Finally it can reply if its own veilid is enabled.
    VeilidEnabled(TypedKey),
    // Invitation messages
    
    /// An invite that can be sent to another node, allowing them to connect.
    Invite(Invite),
    
    /// Request that an invite of the indicated type be sent to the peripheral.
    /// This is so that it can be exchanged to another node to connect.
    GenerateInvite(InviteType),
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

/// Used to indicate which type of invite to generate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InviteType {
    /// A request to create an invite to a chord this node is a member of.
    Chord,
    /// A request to create an invite describing how to connect to the node
    /// through Veilid.
    Veilid,
}

/// An invite to establish a connection to some other base node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Invite {
    /// The invite describes a chord to join
    Chord{},
    /// The invite describes how to use Veilid to communicate.
    Veilid(VeilidInvite)
}

impl Invite {
    /// Convert this invite into a string representation to send to another node.
    pub fn to_base64(&self) -> String {
        // let str = serde_json::to_vec(self).unwrap();
        let str = serde_cbor::to_vec(self).unwrap();
        general_purpose::URL_SAFE_NO_PAD.encode(str)
    }

    /// Convert this invite from a string representation sent from another node.
    pub fn from_base64<T>(s: T) -> Option<Self> where T: AsRef<[u8]>{
        let bytes = general_purpose::URL_SAFE_NO_PAD.decode(s).ok()?;
        // serde_json::from_slice(&bytes).ok()
        serde_cbor::from_slice(&bytes).ok()
    }

    /// Generate the sha 256 hash of this invite
    pub fn sha256(&self) -> String {
        let bytes = serde_cbor::to_vec(self).unwrap();
        sha256::digest(bytes.as_slice())
    }

    /// Returns Some if this invite is a chord invite
    pub fn chord(self) -> Option<()> {
        match self {
            Invite::Chord {  } => Some(()),
            Invite::Veilid(_) => None,
        }
    }

    /// Returns Some([VeilidInvite]) if this invite is a Veilid invite
    pub fn veilid(self) -> Option<VeilidInvite> {
        match self {
            Invite::Chord {  } => None,
            Invite::Veilid(invite) => Some(invite),
        }
    }
}

/// An invite to establish a connection via Veilid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VeilidInvite {
    rel: Relation,
    dht_key: TypedKey,
    code: u64,
    signature: Vec<u8>
}

impl VeilidInvite {
    /// Create a new Veilid invite using the self relation,
    /// the id, and a generated route blob.
    pub fn new(us: &SelfRelation, dht_key: TypedKey, code: u64) -> Self {
        let bytes: Vec<u8> = [
            dht_key.to_string().as_bytes(),
            &code.to_be_bytes()].concat();
        let signature = us.sign(&bytes);
        Self {
            rel: us.relation.clone(),
            dht_key,
            code,
            signature
        }
    }

    /// Returns a reference to the [Relation] the invite is from.
    pub fn rel(&self) -> &Relation {
        &self.rel
    }

    /// Returns a reference to the Veilid Public key of the invite sender.
    pub fn dht_key(&self) -> &TypedKey {
        &self.dht_key
    }

    /// Returns a reference to the blob representation of a [RouteId]
    /// to be imported by Veilid.
    pub fn code(&self) -> u64 {
        self.code
    }

    /// Verify that the Veilid information was sent by the enclosed [Relation].
    pub fn verify(&self) -> bool {
        let bytes: Vec<u8> = [
            self.dht_key.to_string().as_bytes(),
            &self.code.to_be_bytes()].concat();
        self.rel.verify(&bytes, &self.signature)
    }

    /// Convert the Invite to base 64 representation
    pub fn to_base64(&self) -> String {
        let json = serde_json::to_vec(self).unwrap();
        general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    /// Construct an Invite from the base 64 representation
    pub fn from_base64(str: String) -> Option<Self> {
        let slice = general_purpose::URL_SAFE_NO_PAD.decode(str).ok()?;
        serde_json::from_slice(&slice).ok()
    }
}

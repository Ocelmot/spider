#![deny(missing_docs)]

//! The spider_link crate encapsulates everything related to establishing
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

pub(crate) mod error;
pub(crate) use error::{LinkError, LinkResult};

pub mod link;
// pub use link::Link;
pub mod id;
pub mod message;
use id::SpiderId;
pub mod beacon;
mod keyfile;
pub use keyfile::Keyfile;

// TODO: This should be renamed to SpiderId, and the generic id
// renamed to something else.
/// The id used by the spider protocol.
/// The id is a 2048 bit public key, it requires 294 bytes to represent.
pub type SpiderId2048 = SpiderId<294>;

mod relation;
pub use relation::{RelSig, Relation, Role, SelfRelation};

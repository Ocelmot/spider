//! The link module contains the code to establish links between members of the
//! spider network.
//!
//! The [link_set::LinkSet] represents a set of links between two network
//! members. There may be duplicate links, or links using different protocols.
//! Any or all of them will be used to communicate [crate::message::Message]s
//! between the members.
//!
//! A [link::Link] is a trait that defines operations to send and receive
//! messages, and be managed by the [link_set::LinkSet].
//!
//! [tcp_link::TCPLink] is an implementation of [link::Link] that uses TCP.

mod link_set;
pub use link_set::{LinkSet, LinkSetMsg};

mod protocol;
use protocol::LinkProtocol;

mod impls;
pub use impls::TCPLink;
pub use impls::{PipeLink, PipeLinkBuilder, PipeLinkHub};
// pub use impls::{VeilidConnector, VeilidHub, VeilidLink};
use tokio::sync::mpsc::Receiver;

#[cfg(test)]
mod tests;

// Link definitions

use crate::{LinkResult, Relation, SelfRelation};
use std::{future::Future, pin::Pin};

/// The Link trait encapsulates different ways of connecting members of the
/// spider network, so they can be used with [LinkSet]
pub trait Link: Send + Sync {
    /// Returns the [SelfRelation] for this link.
    fn self_relation(&self) -> &SelfRelation;
    /// Returns the [Relation] corresponding to the other side of this link.
    fn other_relation(&self) -> &Relation;

    /// Send a [LinkProtocol] to the other side of the network.
    fn send(&mut self, msg: LinkProtocol) -> impl Future<Output = LinkResult> + Send;
    /// Receive a [LinkProtocol] from the other side.
    fn recv(&mut self) -> impl Future<Output = LinkResult<LinkProtocol>> + Send;
    /// Returns a Receiver of [LinkProtocol]s that will fill with items from the
    /// Link.
    fn take_reader(&mut self) -> LinkResult<Receiver<LinkProtocol>>;

    /// returns the maximum size of data that can be sent through this Link
    ///
    /// The [LinkSet] will automatically break up larger messages during the
    /// serialization and deserialization process to overcome this limit.
    fn max_size(&self) -> u32;

    /// Should the [LinkSet] remove this link. I.E. it will not be able to send
    /// any more data.
    fn is_closed(&mut self) -> bool;
}

/// This trait is a wrapper around [Link] to allow it to be a trait object. Implement [Link] instead of this trait.
///
/// There is a blanket implementation of PinnedLink for all types that implement Link, that pins the returned futures.
#[allow(dead_code)]
pub trait PinnedLink: private::LinkSeal + Send + Sync {
    /// Wrapper around [Link::self_relation]
    fn self_relation(&self) -> &SelfRelation;
    /// Wrapper around [Link::other_relation]
    fn other_relation(&self) -> &Relation;

    /// Wrapper around [Link::send]
    fn send(&mut self, msg: LinkProtocol) -> Pin<Box<dyn Future<Output = LinkResult> + '_ + Send>>;
    /// Wrapper around [Link::recv]
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = LinkResult<LinkProtocol>> + '_ + Send>>;
    /// Wrapper around [Link::take_reader]
    fn take_reader(&mut self) -> LinkResult<Receiver<LinkProtocol>>;

    /// Wrapper around [Link::max_size]
    fn max_size(&self) -> u32;
    /// Wrapper around [Link::is_closed]
    fn is_closed(&mut self) -> bool;
}

impl<T: Link> PinnedLink for T {
    /// Returns the [SelfRelation] for this link.
    fn self_relation(&self) -> &SelfRelation {
        self.self_relation()
    }
    /// Returns the [Relation] corresponding to the other side of this link.
    fn other_relation(&self) -> &Relation {
        self.other_relation()
    }

    fn send(&mut self, msg: LinkProtocol) -> Pin<Box<dyn Future<Output = LinkResult> + '_ + Send>> {
        Box::pin(async { self.send(msg).await })
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = LinkResult<LinkProtocol>> + '_ + Send>> {
        Box::pin(async { self.recv().await })
    }

    fn take_reader(&mut self) -> LinkResult<Receiver<LinkProtocol>> {
        self.take_reader()
    }

    fn max_size(&self) -> u32 {
        self.max_size()
    }

    fn is_closed(&mut self) -> bool {
        self.is_closed()
    }
}

impl<PL: PinnedLink + 'static> From<PL> for Box<dyn PinnedLink> {
    fn from(value: PL) -> Self {
        Box::new(value)
    }
}

mod private {
    pub trait LinkSeal {}

    impl<L: super::Link> LinkSeal for L {}
}

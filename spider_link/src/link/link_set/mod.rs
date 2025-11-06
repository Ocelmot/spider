mod connector;
mod deadline;
pub(crate) use deadline::Deadline;
mod instrument_link;
mod slice_manager;
mod state;

mod core;
use core::LinkSetCore;

pub(crate) trait LinkConnectorInner:
    Fn(
        SelfRelation,
        Relation,
        String,
    ) -> Pin<Box<dyn Future<Output = LinkResult<Box<dyn PinnedLink>>> + Send>>
    + Send
    + Sync
{
}
impl<T> LinkConnectorInner for T where
    T: Fn(
            SelfRelation,
            Relation,
            String,
        ) -> Pin<Box<dyn Future<Output = LinkResult<Box<dyn PinnedLink>>> + Send>>
        + Send
        + Sync
{
}

pub(crate) type LinkConnector = Box<dyn LinkConnectorInner>;

#[cfg(test)]
mod tests;

use super::{Link, PinnedLink};
use crate::{
    error::{ErrorKind, ProblemWrap},
    message::Message,
    LinkError, LinkResult, Relation, SelfRelation,
};
use std::{future::Future, pin::Pin};
use tokio::sync::mpsc::{Receiver, Sender};

/// LinkSet manages a set of Links to another member of the spider network, and
/// transmits messages across them. It also manages connecting and reconnecting.
pub struct LinkSet {
    to_core: Sender<LinkSetControl>,
    from_core: Option<Receiver<LinkSetMsg>>,
    rel: Relation,
    state: Option<u64>,
}

impl LinkSet {
    /// Create a new LinkSet
    pub fn new(self_rel: SelfRelation, rel: Relation) -> Self {
        let (to_core, from_core) = LinkSetCore::start(self_rel, rel.clone());
        Self {
            to_core,
            from_core: Some(from_core),
            rel,
            state: None,
        }
    }

    /// Returns a reference to the relation on the other side of this connection.
    pub fn other_relation(&self) -> &Relation {
        &self.rel
    }

    /// Cause the LinkSet to attempt to connect using stored addresses, if any.
    pub async fn connect(&self) -> LinkResult {
        self.to_core.send(LinkSetControl::Connect).await.wrap()
    }

    /// Cause the LinkSet to disconnect, clearing all active links it may contain
    pub async fn disconnect(&self) -> LinkResult {
        self.to_core.send(LinkSetControl::Disconnect).await.wrap()
    }

    /// Adds a new connector to the link set for the purposes of reestablishing
    /// a broken connection.
    pub async fn add_connector<L, Func, Fut>(&self, conn: Func) -> LinkResult
    where
        L: Link + 'static,
        Func: Fn(SelfRelation, Relation, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = LinkResult<L>> + Send + Sync + 'static,
    {
        let wrapped_con = Box::new(move |sr, r, a| {
            let fut = conn(sr, r, a);
            let f2 = async {
                let link = fut.await;
                let link = link.map(|item| Box::new(item) as Box<dyn PinnedLink + 'static>);
                // Box::new(link) as LinkResult<Box<dyn PinnedLink + 'static>>
                link
            };
            let x = Box::pin(f2) as Pin<Box<dyn Future<Output = LinkResult<_>> + Send>>;
            x
        });
        // Box<dyn for<'a> Fn(&'a str) -> Pin<Box<dyn Future<Output = Box<dyn PinnedLink>> + Send + 'a>> + Send + Sync>
        self.to_core
            .send(LinkSetControl::AddConnector(wrapped_con))
            .await
            .wrap()
    }

    /// Enables or disables the LinkSet auto reconnect feature
    pub async fn set_reconnect(&self, reconnect: bool) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Reconnect(reconnect))
            .await
            .wrap()
    }

    /// Enables or disables the LinkSet auto reconnect feature
    pub async fn set_allow_reconnect(&self, grace_period: Option<u64>) -> LinkResult {
        self.to_core
            .send(LinkSetControl::AllowReconnect(grace_period))
            .await
            .wrap()
    }

    /// Adds an address to the link set to use for auto reconnection
    pub async fn add_addr(&self, addr: String) -> LinkResult {
        self.to_core
            .send(LinkSetControl::AddAddress(addr))
            .await
            .wrap()
    }

    /// Adds an address to the link set to try to use for auto reconnection one time
    pub async fn try_addr(&self, addr: String) -> LinkResult {
        self.to_core
            .send(LinkSetControl::TryAddress(addr))
            .await
            .wrap()
    }

    /// Add a new link to the LinkSet
    pub async fn add_link<L>(&self, link: L) -> LinkResult
    where
        L: Into<Box<dyn PinnedLink>> + 'static,
    {
        self.to_core
            .send(LinkSetControl::AddLink(link.into()))
            .await
            .wrap()
    }

    /// Send a message across the link set
    pub async fn send(&self, msg: Message) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Message(msg, None))
            .await
            .wrap()
    }

    /// Send a message across the link set, using the epoch to prevent this
    /// message to be sent after a reconnection.
    pub async fn send_with_epoch(&self, msg: Message, epoch: u64) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Message(msg, Some(epoch)))
            .await
            .wrap()
    }

    /// Send a message across the link set, using the epoch to prevent this
    /// message to be sent after a reconnection.
    pub async fn send_opt_epoch(&self, msg: Message, epoch: Option<u64>) -> LinkResult {
        self.to_core
            .send(LinkSetControl::Message(msg, epoch))
            .await
            .wrap()
    }

    /// Get a message from the link set
    pub async fn recv(&mut self) -> LinkResult<LinkSetMsg> {
        let ret = self
            .from_core
            .as_mut()
            .ok_or(LinkError::new().problem(ErrorKind::Taken))?
            .recv()
            .await
            .wrap()?;
        if let LinkSetMsg::Connected(epoch) = &ret {
            self.state = Some(*epoch);
        }
        if let LinkSetMsg::Disconnected = &ret {
            self.state = None;
        }
        Ok(ret)
    }

    /// Takes the receiver part of this LinkSet
    pub fn take_recv(&mut self) -> Option<Receiver<LinkSetMsg>> {
        self.state = None;
        self.from_core.take()
    }

    /// Clones the sender part of this LinkSet, the clone will behave as if the receiver was taken.
    pub fn clone_sender(&self) -> Self {
        Self {
            to_core: self.to_core.clone(),
            from_core: None,
            rel: self.rel.clone(),
            state: None,
        }
    }
}

impl ::core::fmt::Debug for LinkSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = if self.from_core.is_some() {
            // has recv
            match self.state {
                Some(epoch) => format!("Connected(epoch={})", epoch),
                None => String::from("Disconnected"),
            }
        } else {
            // recv was taken
            String::from("Taken")
        };

        write!(f, "LinkSet {{ rel: {:?}, status: {} }}", &self.rel, state)
    }
}

enum LinkSetControl {
    Connect,
    Disconnect,
    AddConnector(LinkConnector),
    Reconnect(bool),
    AllowReconnect(Option<u64>),
    AddAddress(String),
    TryAddress(String),
    AddLink(Box<dyn PinnedLink>),
    Message(Message, Option<u64>),
}

/// Indicates changes to the status of the LinkSet, and if a message was
/// received.
#[derive(Debug)]
pub enum LinkSetMsg {
    /// The LinkSet has disconnected
    Disconnected,

    /// The LinkSet successfully established a connection
    Connected(u64),

    /// The LinkSet has started or stopped reconnecting.
    /// 
    /// A reconnection allows the connection to be reestablished within a
    /// certain time period without causing a Disconnected/Connected cycle.
    /// However, it may still be useful to know if the LinkSet is attempting a
    /// reconnect to provide it with new addresses to help its attempt.
    /// 
    /// These messages are not sent unless the LinkSet's reconnect feature is
    /// enabled.
    Connecting(bool),

    /// A [Message] was received with the given epoch
    Message(Message, u64),
}

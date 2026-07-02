//! Maps the underlying Link implementations to a fully authenticated link
//! identified with its relation.
//!
//! Provides connectors that function with LinkSet as well as listeners to
//! provide new connections.


use tokio::{sync::mpsc::Sender, task::JoinHandle};

use crate::{LinkResult, SelfRelation, link_impls::authenticated::Authenticated};

#[cfg(feature = "transport_tcp")]
pub mod tcp;

#[cfg(feature = "transport_iroh")]
pub mod iroh;

/// LinkListeners start listen tasks that generate links for a particular scheme
pub trait LinkListener {
    /// The scheme this listener produces
    fn scheme(&self) -> &'static str;
    /// Creates a listen task that emits [Authenticated] links through the
    /// provided sender
    fn listen(&self, sr: SelfRelation, sender: Sender<Authenticated>)
        -> JoinHandle<LinkResult<()>>;
}

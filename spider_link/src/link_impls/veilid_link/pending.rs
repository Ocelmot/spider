use tokio::sync::oneshot;
use veilid_core::TypedKey;

use crate::Relation;

use super::VeilidLink;

pub(crate) struct OutgoingPending {
    /// The relation of the other side of the connection
    to_rel: Relation,
    /// The dht to which to send the introduction
    to_dht: TypedKey,
    /// The dht key to convey to the other side of the connection
    dht_key: TypedKey,

    sender: oneshot::Sender<VeilidLink>,
}

impl OutgoingPending {
    pub fn new(
        to_rel: Relation,
        to_dht: TypedKey,
        dht_key: TypedKey,
        sender: oneshot::Sender<VeilidLink>,
    ) -> Self {
        Self {
            to_rel,
            to_dht,
            dht_key,
            sender,
        }
    }

    pub fn send(self, link: VeilidLink) {
        let _ = self.sender.send(link);
    }
}

pub(crate) struct IncomingPending {
    /// The relation of the other side of the connection
    to_rel: Relation,
    /// The dht to which to send the introduction
    to_dht: TypedKey,
    /// The dht key to convey to the other side of the connection
    dht_key: TypedKey,
}

impl IncomingPending {
    pub fn new(
        to_rel: Relation,
        to_dht: TypedKey,
        dht_key: TypedKey,
    ) -> Self {
        Self {
            to_rel,
            to_dht,
            dht_key,
        }
    }
}

pub(crate) trait Introducible {
    fn to_rel(&self) -> &Relation;
    fn to_dht(&self) -> &TypedKey;
    fn dht_key(&self) -> &TypedKey;
}

impl Introducible for OutgoingPending {
    fn to_rel(&self) -> &Relation {
        &self.to_rel
    }

    fn to_dht(&self) -> &TypedKey {
        &self.to_dht
    }

    fn dht_key(&self) -> &TypedKey {
        &self.dht_key
    }
}

impl Introducible for IncomingPending {
    fn to_rel(&self) -> &Relation {
        &self.to_rel
    }

    fn to_dht(&self) -> &TypedKey {
        &self.to_dht
    }

    fn dht_key(&self) -> &TypedKey {
        &self.dht_key
    }
}

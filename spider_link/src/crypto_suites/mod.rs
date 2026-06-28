use crate::{LinkError, crypto_suites::ciphers::{Opener, Sealer}};

pub (crate) mod ciphers;

#[cfg(feature = "cs_0_x25519_chacha20")]
mod x25519_chacha20;
#[cfg(feature = "cs_0_x25519_chacha20")]
use x25519_chacha20::X25519ChaCha20;


#[auto_const_array::auto_const_array_attr]
const SUITES: [&'static dyn Suite; _] = [
    #[cfg(feature = "cs_0_x25519_chacha20")]
    &X25519ChaCha20{}
];


const _: () = assert!(SUITES.len() > 0, "At least one crypto suite must be enabled through the crate's feature flags");


pub fn by_id(id: u8) -> Option<&'static dyn Suite> {
    SUITES.iter().copied().find(|s| s.id() == id)
}
pub fn offer(floor_rank: u8) -> impl Iterator<Item = &'static dyn Suite> + 'static {
    SUITES.iter().copied().filter(move |s| s.rank() >= floor_rank)
}

pub fn select_offer(offers: &[u8], floor_rank: u8) -> Option<&'static dyn Suite> {
    for offer in offers{
        let Some(offer) = by_id(*offer) else {continue;};
        if offer.rank() < floor_rank {
            continue;
        }
        return Some(offer);
    }
    None
}

pub trait Suite: Sync {
    fn id(&self) -> u8;
    fn rank(&self) -> u8;
    /// Begin a handshake; returns a driver holding ephemeral state +
    /// the local key-agreement bytes to put in the hello.
    fn start(&self, role: HandshakeRole) -> Box<dyn Handshake>;
}

pub trait Handshake: Send + Sync {
    fn local_kex(&self) -> &[u8];
    fn finish(self: Box<Self>, peer_kex: &[u8])
        -> Result<(Box<dyn Sealer>, Box<dyn Opener>), LinkError>;
}

#[derive(Debug, Clone, Copy)]
pub enum HandshakeRole{
    Initiator,
    Responder,
}

impl HandshakeRole{
    /// Returns (send, recv)
    pub fn order<Item>(&self, itor: Item, rtoi: Item) -> (Item, Item) {
        match self {
            HandshakeRole::Initiator => (itor, rtoi),
            HandshakeRole::Responder => (rtoi, itor),
        }
    }

    pub fn tag(&self) -> &'static [u8] {
        match self {
            HandshakeRole::Initiator => b"I",
            HandshakeRole::Responder => b"R",
        }
    }

    pub fn other(&self) -> Self {
        match self {
            HandshakeRole::Initiator => HandshakeRole::Responder,
            HandshakeRole::Responder => HandshakeRole::Initiator,
        }
    }
}
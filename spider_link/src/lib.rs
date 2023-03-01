use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
    RsaPrivateKey,
};
use serde::{Deserialize, Serialize};
use tokio::{net::ToSocketAddrs, sync::mpsc::Receiver};

pub mod link;
pub mod message;
pub use link::Link;
pub mod id;
use id::SpiderId;
pub mod beacon;

pub type SpiderId2048 = SpiderId<294>; // 2048 bit pub key takes 294 bytes

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Role {
    Peripheral,
    Peer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub role: Role,
    pub id: SpiderId2048,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfRelation {
    pub priv_key_der: Vec<u8>,
    pub relation: Relation,
}

impl SelfRelation {
    pub fn from_key(key: RsaPrivateKey, role: Role) -> Self {
        let priv_bytes = key.to_pkcs8_der().unwrap().as_ref().to_vec();
        let pub_bytes = key.to_public_key().to_public_key_der().unwrap();
        let id = SpiderId::from_bytes(pub_bytes.as_ref().try_into().unwrap());
        Self {
            priv_key_der: priv_bytes,
            relation: Relation { id, role },
        }
    }
    pub fn from_der(bytes: &[u8], role: Role) -> Self {
        let key = RsaPrivateKey::from_pkcs8_der(bytes).unwrap();
        Self::from_key(key, role)
    }

    pub fn generate_key(role: Role) -> Self {
        let mut rng = rand::thread_rng();
        let key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
        Self::from_key(key, role)
    }

    pub fn private_key(&self) -> RsaPrivateKey {
        RsaPrivateKey::from_pkcs8_der(&self.priv_key_der).unwrap()
    }

    // connect: to connect to a particular address
    pub async fn connect_to<A: ToSocketAddrs>(&self, addr: A, relation: Relation) -> Option<Link> {
        Link::connect(self.clone(), addr, relation).await
    }
    // listener: to generate new connections
    pub fn listen<A: ToSocketAddrs + Send + 'static>(&self, addr: A) -> Receiver<Link> {
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
use std::sync::Arc;

use base64::{engine::general_purpose, Engine};
use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
    RsaPrivateKey,
};
use serde::{Deserialize, Serialize};
use tokio::{net::ToSocketAddrs, sync::{mpsc::Receiver, Mutex}};

pub mod link;
pub mod message;
pub use link::Link;
pub mod id;
use id::SpiderId;
pub mod beacon;

pub type SpiderId2048 = SpiderId<294>; // 2048 bit pub key takes 294 bytes

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Role {
    Peripheral,
    Peer,
}

#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
pub struct Relation {
    pub role: Role,
    pub id: SpiderId2048,
}

impl Relation{
    pub fn is_peripheral(&self) -> bool{
        if let Role::Peripheral = self.role{
            true
        }else{
            false
        }
    }

    pub fn is_peer(&self) -> bool{
        if let Role::Peer = self.role{
            true
        }else{
            false
        }
    }

    pub fn to_base64(&self) -> String {
        let role: u8 = match self.role {
            Role::Peripheral => 1,
            Role::Peer => 0,
        };
        let mut bytes = self.id.clone().to_bytes().to_vec();
        bytes.push(role);
        general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }
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
use id::SpiderId;
use link::Link;
use rsa::{RsaPrivateKey, pkcs8::{EncodePublicKey, DecodePrivateKey, EncodePrivateKey}};
use tokio::{net::ToSocketAddrs, sync::mpsc::Receiver};
use serde::{Serialize, Deserialize};

pub mod message;
pub mod link;
pub mod id;


type SpiderId2048 = SpiderId<294>; // 2048 bit pub key takes 294 bytes

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role{
	Member,
	Peripheral,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation{
	pub id: SpiderId2048,
	pub role: Role,
}

#[derive(Debug, Clone)]
pub struct SelfRelation{
	pub priv_key_der: Vec<u8>,
	pub relation: Relation,
}

impl SelfRelation {
	pub fn from_key(key: RsaPrivateKey, role: Role) -> Self {
        let priv_bytes = key.to_pkcs8_der().unwrap().as_ref().to_vec();
		let pub_bytes = key.to_public_key().to_public_key_der().unwrap();
		let id = SpiderId::from_bytes(pub_bytes.as_ref().try_into().unwrap());
		Self{
			priv_key_der: priv_bytes,
			relation: Relation { id, role },
		}
	}
    pub fn from_der(bytes: &[u8], role: Role) -> Self {
        let key = RsaPrivateKey::from_pkcs8_der(bytes).unwrap();
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





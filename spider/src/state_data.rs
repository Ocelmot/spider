use serde::{Deserialize, Serialize};
use serde_with::{serde_as, base64::Base64};
use spider_link::{message::DirectoryEntry, Relation, Role, SelfRelation, SpiderId2048};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
// use veilid_core::DHTRecordDescriptor;

use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
    RsaPrivateKey,
};

use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};

#[derive(Debug, Clone)]
pub struct StateData {
    // Acquire locks in struct order.
    filename: Arc<Mutex<PathBuf>>,
    inner: Arc<Mutex<StateDataInner>>,
}

impl StateData {
    pub fn load_file(path: &Path) -> io::Result<Self> {
        let data = fs::read_to_string(&path)?;
        let inner = serde_json::from_str(&data).expect("Failed to deserialize config");
        Ok(Self {
            filename: Arc::new(Mutex::new(path.to_path_buf())),
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    pub fn with_generated_key(path: &Path) -> Self {
        let path = path.to_path_buf();
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
        let bytes = priv_key.to_pkcs8_der().unwrap().as_bytes().to_vec();
        Self {
            filename: Arc::new(Mutex::new(path)),
            inner: Arc::new(Mutex::new(StateDataInner::new(bytes))),
        }
    }

    pub async fn save_file(&self) {
        let filename = self.filename.lock().await;
        let inner = self.inner.lock().await;
        let contents = serde_json::to_string(&*inner).unwrap();
        tokio::fs::write(&*filename, contents).await;
    }

    pub async fn priv_key(&self) -> RsaPrivateKey {
        let inner = self.inner.lock().await;
        let priv_key = RsaPrivateKey::from_pkcs8_der(&inner.key_der).unwrap();
        priv_key
    }

    pub async fn self_id(&self) -> SpiderId2048 {
        let priv_key = self.priv_key().await;
        let pub_key = priv_key.to_public_key();
        SpiderId2048::from_key(pub_key)
    }

    pub async fn self_relation(&self) -> SelfRelation {
        let key = self.priv_key().await;
        SelfRelation::from_key(key, Role::Peer)
    }

    // Peripheral Items
    pub async fn peripheral_services(&self) -> MappedMutexGuard<'_, HashMap<String, bool>> {
        let inner = self.inner.lock().await;
        MutexGuard::map(inner, |f| &mut f.peripheral_services)
    }

    // Router Items
    pub async fn name(&self) -> MappedMutexGuard<'_, String> {
        let inner = self.inner.lock().await;
        // inner.name.as_ref().unwrap_or(&String::from("No Name"))
        MutexGuard::map(inner, |i| i.name.get_or_insert(String::from("NoName")))
    }

    pub async fn load_directory(&self) -> HashMap<Relation, DirectoryEntry> {
        let inner = self.inner.lock().await;
        let mut ret = HashMap::new();
        for entry in &inner.directory {
            let rel = entry.relation().clone();
            ret.insert(rel, entry.clone());
        }
        ret
    }
    pub async fn save_directory(&self, directory: &HashMap<Relation, DirectoryEntry>) {
        let mut v = Vec::with_capacity(directory.len());
        for (_, entry) in directory {
            v.push(entry.clone());
        }
        let mut inner = self.inner.lock().await;
        inner.directory = v;
    }

    // Transport related data

    pub async fn iroh_secret(&self) -> MappedMutexGuard<'_, [u8; 32]> {
        let inner = self.inner.lock().await;
        MutexGuard::map(inner, |f| &mut f.iroh_secret)
    }

    // pub async fn veilid_own_dht(&self) -> MappedMutexGuard<'_, Option<()>> {
    //     let inner = self.inner.lock().await;
    //     // MutexGuard::map(inner, |i| &mut i.veilid_own_dht)
    //     MutexGuard::map(inner, |i| None)
    // }

}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
struct StateDataInner {
    pub key_der: Vec<u8>,

    // Peripheral Items
    #[serde(default)]
    pub peripheral_services: HashMap<String, bool>,

    // Router Items
    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    directory: Vec<DirectoryEntry>,

    // Transport config items
    #[serde_as(as = "Base64")]
    #[serde(default = "iroh_default")]
    iroh_secret: [u8; 32],

    // #[serde(default)]
    // veilid_own_dht: Option<DHTRecordDescriptor>,
}

impl StateDataInner {
    fn new(key_der: Vec<u8>) -> Self {
        Self {
            key_der,

            // Peripheral Items
            peripheral_services: HashMap::new(),

            // Router Items
            name: None,
            
            directory: Vec::new(),

            // Transport items
            iroh_secret: iroh_default(),

            // veilid_own_dht: None,
        }
    }
}

fn iroh_default() -> [u8; 32]{
    use rand::RngCore;

    let mut new_secret = [0u8; 32];
    let mut rng = rand::rngs::OsRng;
    rng.fill_bytes(&mut new_secret);
    new_secret
}

use serde::{Deserialize, Serialize};
use serde_with::{base64::Base64, serde_as};
use spider_link::{
    discovery::BaseAdvert, link_set::links::Address, message::DirectoryEntry, Relation, Role,
    SelfRelation, SpiderId2048,
};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tracing::warn;

use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
    RsaPrivateKey,
};

use tokio::sync::{
    watch::{self, Receiver},
    Mutex, RwLock, RwLockMappedWriteGuard, RwLockReadGuard, RwLockWriteGuard,
};

#[derive(Debug, Clone)]
pub struct StateData {
    // Acquire locks in struct order.
    dirty: Arc<AtomicBool>,
    filename: Arc<Mutex<PathBuf>>,
    inner: Arc<RwLock<StateDataInner>>,
    advert_template: watch::Sender<BaseAdvert>,
    static_addrs: Vec<Address>,
}

impl StateData {
    pub fn load_file(path: &Path, static_addrs: Vec<Address>) -> io::Result<Self> {
        let data = fs::read_to_string(&path)?;
        let inner = serde_json::from_str(&data).expect("Failed to deserialize config");
        let template = generate_template(&inner, &static_addrs);
        Ok(Self {
            dirty: Arc::new(AtomicBool::new(false)),
            filename: Arc::new(Mutex::new(path.to_path_buf())),
            inner: Arc::new(RwLock::new(inner)),
            advert_template: watch::Sender::new(template),
            static_addrs,
        })
    }

    pub fn with_generated_key(path: &Path, static_addrs: Vec<Address>) -> Self {
        let path = path.to_path_buf();
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
        let bytes = priv_key.to_pkcs8_der().unwrap().as_bytes().to_vec();
        let inner = StateDataInner::new(bytes);
        let template = generate_template(&inner, &static_addrs);
        Self {
            dirty: Arc::new(AtomicBool::new(true)),
            filename: Arc::new(Mutex::new(path)),
            inner: Arc::new(RwLock::new(inner)),
            advert_template: watch::Sender::new(template),
            static_addrs,
        }
    }

    pub async fn save_file(&self) {
        let was_dirty = self.dirty.swap(false, Ordering::SeqCst);
        if was_dirty {
            let filename = self.filename.lock().await;
            let inner = self.inner.read().await;
            let contents = serde_json::to_string(&*inner).unwrap();
            if let Err(e) = tokio::fs::write(&*filename, contents).await {
                warn!("Failed to save state file: {e}");
            }
        }
    }

    pub fn advert_template_subscribe(&self) -> Receiver<BaseAdvert> {
        self.advert_template.subscribe()
    }

    pub async fn priv_key(&self) -> RsaPrivateKey {
        let inner = self.inner.read().await;
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

    pub async fn set_beacon_emit_name(&self, emit: bool) {
        {
            let mut inner = self.inner.write().await;
            inner.beacon_emit_name = emit;
        }
        self.dirty.store(true, Ordering::SeqCst);
        self.generate_template().await;
    }

    pub async fn set_beacon_emit_id(&self, emit: bool) {
        {
            let mut inner = self.inner.write().await;
            inner.beacon_emit_id = emit;
        }
        self.dirty.store(true, Ordering::SeqCst);
        self.generate_template().await;
    }

    // Peripheral Items
    pub async fn peripheral_services(&self) -> RwLockReadGuard<'_, HashMap<String, bool>> {
        let inner = self.inner.read().await;
        RwLockReadGuard::map(inner, |f| &f.peripheral_services)
    }

    pub async fn modify_peripheral_services<F>(&self, mut func: F)
    where
        F: FnMut(RwLockMappedWriteGuard<'_, HashMap<String, bool>>),
    {
        let inner = self.inner.write().await;
        let guard = RwLockWriteGuard::map(inner, |f| &mut f.peripheral_services);
        func(guard);
        self.dirty.store(true, Ordering::SeqCst);
    }

    // Router Items
    pub async fn name(&self) -> RwLockReadGuard<'_, str> {
        let inner = self.inner.read().await;
        RwLockReadGuard::map(inner, |i| i.name.as_deref().unwrap_or("NoName"))
    }

    pub async fn set_name(&self, name: String) {
        {
            let mut inner = self.inner.write().await;
            inner.name = Some(name);
        }
        self.generate_template().await;
        self.dirty.store(true, Ordering::SeqCst);
    }

    pub async fn load_directory(&self) -> HashMap<Relation, DirectoryEntry> {
        let inner = self.inner.read().await;
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
        let mut inner = self.inner.write().await;
        inner.directory = v;
    }

    // Transport related data

    pub async fn iroh_secret(&self) -> RwLockReadGuard<'_, [u8; 32]> {
        let inner = self.inner.read().await;
        RwLockReadGuard::map(inner, |f| &f.iroh_secret)
    }

    /// Regenerates the template used by the discovery services
    async fn generate_template(&self) {
        let template = generate_template(&*self.inner.read().await, &self.static_addrs);
        self.advert_template.send_replace(template);
    }
}

/// Generate the template from the inner and the static addrs. Does not hold locks or is async
fn generate_template(inner: &StateDataInner, static_addrs: &[Address]) -> BaseAdvert {
    let addrs = static_addrs.to_vec();

    let (name, key_der) = {
        (
            inner.beacon_emit_name.then(|| inner.name.clone()).flatten(),
            inner.beacon_emit_id.then(|| inner.key_der.clone()),
        )
    };

    let id = key_der.map(|der| {
        let priv_key = RsaPrivateKey::from_pkcs8_der(&der).unwrap();
        SpiderId2048::from_key(priv_key.to_public_key())
    });

    BaseAdvert { addrs, name, id }
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
struct StateDataInner {
    key_der: Vec<u8>,

    #[serde(default = "default_true")]
    beacon_emit_name: bool,
    #[serde(default = "default_true")]
    beacon_emit_id: bool,

    // Peripheral Items
    #[serde(default)]
    peripheral_services: HashMap<String, bool>,

    // Router Items
    #[serde(default)]
    name: Option<String>,

    #[serde(default)]
    directory: Vec<DirectoryEntry>,

    // Transport config items
    #[serde_as(as = "Base64")]
    #[serde(default = "iroh_default")]
    iroh_secret: [u8; 32],
}

impl StateDataInner {
    fn new(key_der: Vec<u8>) -> Self {
        Self {
            key_der,
            beacon_emit_name: true,
            beacon_emit_id: true,

            // Peripheral Items
            peripheral_services: HashMap::new(),

            // Router Items
            name: None,

            directory: Vec::new(),

            // Transport items
            iroh_secret: iroh_default(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn iroh_default() -> [u8; 32] {
    use rand::RngCore;

    let mut new_secret = [0u8; 32];
    let mut rng = rand::rngs::OsRng;
    rng.fill_bytes(&mut new_secret);
    new_secret
}

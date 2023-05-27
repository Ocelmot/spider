use std::{fs, path::{Path, PathBuf}, io, sync::Arc, collections::HashMap};
use spider_link::{SpiderId2048, SelfRelation, Role};
use serde::{Serialize, Deserialize};
use lru::LruCache;

use rsa::{RsaPrivateKey, pkcs8::{DecodePrivateKey, EncodePrivateKey}};



use tokio::sync::{Mutex, MutexGuard, MappedMutexGuard};




#[derive(Debug, Clone)]
pub struct StateData{
    // Aquire locks in struct order.
    filename: Arc<Mutex<PathBuf>>,
    inner: Arc<Mutex<StateDataInner>>,
}


impl StateData {


    pub fn load_file(path: &Path) -> io::Result<Self> {
        let data = fs::read_to_string(&path)?;
		let inner = serde_json::from_str(&data).expect("Failed to deserialize config");
        Ok(Self{
            filename: Arc::new(Mutex::new(path.to_path_buf())),
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    pub fn with_generated_key(path: &Path) -> Self{
        let path = path.to_path_buf();
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
        let bytes = priv_key.to_pkcs8_der().unwrap().as_ref().to_vec();
        StateData{
            filename: Arc::new(Mutex::new(path)),
            inner: Arc::new(Mutex::new(StateDataInner::new(bytes))),
        }
    }

    pub async fn save_file(&self){
        let filename = self.filename.lock().await;
        let inner = self.inner.lock().await;
        let contents = serde_json::to_string(&*inner).unwrap();
        tokio::fs::write(&*filename, contents).await;
    }


    pub async fn priv_key(&self) -> RsaPrivateKey{
        let inner = self.inner.lock().await;
        let priv_key = RsaPrivateKey::from_pkcs8_der(&inner.key_der).unwrap();
        priv_key
    }

    pub async fn self_id(&self) -> SpiderId2048{
        let priv_key = self.priv_key().await;
        let pub_key = priv_key.to_public_key();
        SpiderId2048::from_key(pub_key)
    }

    pub async fn self_relation(&self) -> SelfRelation{
        let key = self.priv_key().await;
        SelfRelation::from_key(key, Role::Peer)
    }

    // Pheripheral items
    pub async fn peripheral_services(&self) -> MappedMutexGuard<'_, HashMap<std::string::String, bool>> {
        let inner = self.inner.lock().await;
        MutexGuard::map(inner, |f| &mut f.peripheral_services)
    }

}



#[derive(Debug, Serialize, Deserialize)]
struct StateDataInner{
    pub key_der: Vec<u8>,
    
    // Peripheral items
    #[serde(default)]
    pub peripheral_services: HashMap<String, bool>,
}


impl StateDataInner {
    fn new(key_der: Vec<u8>) -> Self{
        Self{
            key_der,

            // Peripheral items
            peripheral_services: HashMap::new(),
        }
    }
}

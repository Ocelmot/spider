use std::{fs, path::{Path, PathBuf}, io, sync::Arc, collections::HashMap};
use spider_link::{SpiderId2048, SelfRelation, Role, Relation, message::DirectoryEntry};
use serde::{Serialize, Deserialize};

use rsa::{RsaPrivateKey, pkcs8::{DecodePrivateKey, EncodePrivateKey}};



use tokio::sync::{Mutex, MutexGuard, MappedMutexGuard};

use crate::processor::ChordState;




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

    // Pheripheral Items
    pub async fn peripheral_services(&self) -> MappedMutexGuard<'_, HashMap<String, bool>> {
        let inner = self.inner.lock().await;
        MutexGuard::map(inner, |f| &mut f.peripheral_services)
    }

    // Router Items
    pub async fn name(&self) -> MappedMutexGuard<'_, String>{
        let inner = self.inner.lock().await;
        // inner.name.as_ref().unwrap_or(&String::from("No Name"))
        MutexGuard::map(inner, |i|{
            i.name.get_or_insert(String::from("NoName"))
        })
    }
    pub async fn chord_names(&self) -> Vec<String>{
        let inner = self.inner.lock().await;
        inner.chords.keys().cloned().collect()
    }
    pub async fn get_chord(&self, name: &String) -> Option<ChordState>{
        let inner = self.inner.lock().await;

        match inner.chords.get(name){
            Some((listen_addr, pub_addr, advert_addr, addrs)) => {
                let listen_addr = listen_addr.to_string();
                let pub_addr = pub_addr.to_string();
                let advert_addr = advert_addr.to_string();
                let mut state = ChordState::new(listen_addr, pub_addr, advert_addr);
                state.add_addrs(addrs.clone());
                Some(state)
            },
            None => None,
        }
    }

    pub async fn put_chord(&mut self, name: &String, chord: &ChordState){
        let mut inner = self.inner.lock().await;

        let listen_addr = chord.listen_addr.clone();
        let pub_addr = chord.pub_addr.clone();
        let advert_addr = chord.advert_addr.clone();
        let addrs = chord.get_addrs().map(|x|{x.0.clone()}).collect();
        let state = (listen_addr, pub_addr, advert_addr, addrs);
        inner.chords.insert(name.clone(), state);
    }
    pub async fn remove_chord(&mut self, name: &String){
        let mut inner = self.inner.lock().await;

        inner.chords.remove(name);
    }

    pub async fn load_directory(&mut self) -> HashMap<Relation, DirectoryEntry>{
        let inner = self.inner.lock().await;
        let mut ret = HashMap::new();
        for entry in &inner.directory{
            let rel = entry.relation().clone();
            ret.insert(rel, entry.clone());
        }
        ret
    }
    pub async fn save_directory(&mut self, directory: &HashMap<Relation, DirectoryEntry>) {
        let mut v = Vec::with_capacity(directory.len());
        for (_, entry) in directory {
            v.push(entry.clone());
        }
        let mut inner = self.inner.lock().await;
        inner.directory = v;
    }
}



#[derive(Debug, Serialize, Deserialize)]
struct StateDataInner{
    pub key_der: Vec<u8>,
    
    // Peripheral Items
    #[serde(default)]
    pub peripheral_services: HashMap<String, bool>,

    // Router Items
    /// Map from chord names to listen_adder, pub_addr, and vectors of recent addresses
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    chords: HashMap<String, (String, String, String, Vec<String>)>,
    #[serde(default)]
    directory: Vec<DirectoryEntry>,
}


impl StateDataInner {
    fn new(key_der: Vec<u8>) -> Self{
        Self{
            key_der,

            // Peripheral Items
            peripheral_services: HashMap::new(),

            // Router Items
            name: None,
            chords: HashMap::new(),
            directory: Vec::new(), 
        }
    }
}

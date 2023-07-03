use std::{fs, path::Path};

use serde::{Serialize, Deserialize};
use spider_link::{SelfRelation, Relation, Role};



#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SpiderClientState{
    // Identity
    pub self_relation: SelfRelation,
    pub host_relation: Option<Relation>,

    // Address finding
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub strategies: Vec<AddressStrategy>,
    pub last_addr: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub chord_list: Vec<String>,

    pub connection_attempts: u8,
    pub auto_reconnect: bool,
}


impl SpiderClientState {

    pub fn new() -> Self {
        Self{
            self_relation: SelfRelation::generate_key(Role::Peripheral),
            host_relation: None,
            
            strategies: Vec::new(),
            last_addr: None,
            chord_list: Vec::new(),
            
            connection_attempts: 0,
            auto_reconnect: false,
        }
    }

    pub fn from_file(path: &Path) -> Self {
        let data = fs::read_to_string(&path).expect(&format!("Failed to read spider config from file: {:?}", path));
		let config = serde_json::from_str(&data).expect("Failed to deserialize chord state");
        config
    }

    pub fn to_file(&self, path: &Path){
        let data = serde_json::to_string(self).expect("Failed to serialize chord state");
        fs::write(&path, data).expect(&format!("Failed to write spider config to file: {:?}", path));
    }

}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AddressStrategy{
    Localhost,
    LastAddr,
    Addr(String),
    Beacon,
    Chord,
}
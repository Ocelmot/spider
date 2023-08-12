use std::{fs, path::Path, net::SocketAddr};

use serde::{Serialize, Deserialize};
use spider_link::{SelfRelation, Relation, Role};



#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpiderClientState{
    // Identity
    pub self_relation: SelfRelation,
    pub host_relation: Option<Relation>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub permission_code: Option<String>,

    // Config
    pub auto_reconnect: bool,
    pub connection_attempts: u8,

    // Address finding strategies
    // Last known address
    pub last_addr_enable: bool,
    pub last_addr_global: Option<String>,
    pub last_addr_local: Option<String>,

    // Beacon 
    pub beacon_enable: bool,

    // Chord
    pub chord_enable: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub chord_addrs: Vec<String>,

    // Fixed Addresses
    pub fixed_addr_enable: bool,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fixed_addrs: Vec<String>,
}


impl SpiderClientState {

    pub fn new() -> Self {
        Self{
            // Identity
            self_relation: SelfRelation::generate_key(Role::Peripheral),
            host_relation: None,
            permission_code: None,

            // Config
            auto_reconnect: false,
            connection_attempts: 0,

            // Address finding strategies
            // Last known address
            last_addr_enable: true,
            last_addr_global: None,
            last_addr_local: None,

            // Beacon 
            beacon_enable: true,

            // Chord
            chord_enable: true,
            chord_addrs: Vec::new(),

            // Fixed Addresses
            fixed_addr_enable: false,
            fixed_addrs: Vec::new(),
        }
    }

    pub fn from_string(s: String) -> Option<Self> {
        serde_json::from_str(&s).ok()
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

    pub fn set_last_addr(&mut self, set: Option<String>){
        match set {
            Some(addr_str) => {
                match addr_str.parse::<SocketAddr>(){
                    Ok(addr) => {
                        if ip_rfc::global(&addr.ip()){
                            self.last_addr_global = Some(addr_str);
                        }else{
                            self.last_addr_local = Some(addr_str);
                        }
                    },
                    Err(_) => return,
                }
            },
            None => {
                self.last_addr_global = None;
                self.last_addr_local = None;
            },
        }
    }

}

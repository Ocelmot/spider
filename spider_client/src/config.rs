use std::{fs, path::Path};

use serde::{Serialize, Deserialize};





#[derive(Debug, Serialize, Deserialize)]
pub struct SpiderClientConfig{
    pub node_id: u32,
    pub host_addr_override: Option<String>,
    pub addr_list: Option<Vec<String>>,
    pub log_path: Option<String>,

}


impl SpiderClientConfig {

    pub fn new() -> Self {
        Self{
            node_id: 0,
            host_addr_override: None,
            addr_list: None,
            log_path: None,
        }
    }


    pub fn from_file(path: &Path) -> Self {
        let data = fs::read_to_string(&path).expect(&format!("Failed to read spider config from file: {:?}", path));
		let config = serde_json::from_str(&data).expect("Failed to deserialize chord state");
        config
    }


}
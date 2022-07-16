use std::{fs, path::Path};

use serde::{Serialize, Deserialize};





#[derive(Debug, Serialize, Deserialize)]
pub struct SpiderConfig{
    pub id: u32,
    pub listen_addr: String,
    pub pub_addr: String,

    pub local_ids: Vec<u32>,
    pub chord_file: Option<String>,
    pub log_path: Option<String>,
}


impl SpiderConfig {


    pub fn from_file(path: &Path) -> Self {
        let data = fs::read_to_string(&path).expect(&format!("Failed to read config file: {:?}", path));
		let config = serde_json::from_str(&data).expect("Failed to deserialize config");
        config
    }

}



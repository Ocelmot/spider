use std::{fs, path::{Path, PathBuf}};

use serde::{Serialize, Deserialize};





#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpiderConfig{
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_log_path")]
    pub log_path: String,
    #[serde(default = "default_state_data_path")]
    pub state_data_path: String,

    #[serde(default)]
    pub keyfile_path: Option<String>,

    // Router configurations
    // List<Router config>

    // No peripheral configurations

    // UI Config
    // UIConfig

    // Dataset configuration
    #[serde(default)]
    dataset_path: Option<String>,
}


impl SpiderConfig {
    pub fn from_file(path: &Path) -> Self {
        let data = match fs::read_to_string(&path){
            Ok(str) => str,
            Err(_) => String::from("{}"),
        };
        // let data = fs::read_to_string(&path).expect(&format!("Failed to read config file: {:?}", path));
		let config = serde_json::from_str(&data).expect("Failed to deserialize config");
        config
    }

    pub fn dataset_path(&self)-> PathBuf{
        let s = self.dataset_path.clone().unwrap_or(String::from("datasets"));
        PathBuf::from(s)
    }
}




// Defaults
fn default_listen_addr() -> String {
    "0.0.0.0:1930".into()
}

fn default_log_path() -> String {
    "spider.log".into()
}

fn default_state_data_path() -> String {
    "state.dat".into()
}
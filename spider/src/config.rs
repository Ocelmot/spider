use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpiderConfig {
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default = "default_pub_addr")]
    pub pub_addr: String,
    #[serde(default = "default_log_path")]
    pub log_path: String,
    #[serde(default = "default_state_data_path")]
    pub state_data_path: String,

    #[serde(default)]
    pub keyfile_path: Option<String>,

    #[serde(default = "default_true")]
    enable_key_req: bool,

    #[serde(default = "default_true")]
    enable_beacon: bool,

    #[serde(default = "default_beacon_port")]
    beacon_port: u16,

    // No peripheral configurations
    #[serde(default)]
    peripheral_path: Option<String>,

    // UI Config

    // Dataset configuration
    #[serde(default)]
    dataset_path: Option<String>,

    #[serde(default)]
    group_path: Option<String>,

    // Router configuration
    veilid_enabled: Option<bool>,
}

impl SpiderConfig {
    pub fn from_file(path: &Path) -> Self {
        let data = match fs::read_to_string(&path) {
            Ok(str) => str,
            Err(e) => {
                eprintln!("Error loading config, using defaults. Error: {e}");
                String::from("{}")
            },
        };
        // println!("Loaded config data: {}", data);
        // let data = fs::read_to_string(&path).expect(&format!("Failed to read config file: {:?}", path));
        let config = serde_json::from_str(&data).expect("Failed to deserialize config");
        config
    }

    pub fn peripheral_path(&self) -> PathBuf {
        let s = self
            .peripheral_path
            .clone()
            .unwrap_or(String::from("peripherals"));
        PathBuf::from(s)
    }

    pub fn dataset_path(&self) -> PathBuf {
        let s = self
            .dataset_path
            .clone()
            .unwrap_or(String::from("datasets"));
        PathBuf::from(s)
    }

    pub fn group_path(&self) -> PathBuf {
        let s = self.dataset_path.clone().unwrap_or(String::from("groups"));
        PathBuf::from(s)
    }

    pub fn veilid_enabled(&self) -> bool {
        self.veilid_enabled.unwrap_or_else(default_veilid_enabled)
    }

    pub fn key_req_enabled(&self) -> bool {
        self.enable_key_req
    }

    pub fn beacon_enabled(&self) -> bool {
        self.enable_beacon
    }

    pub fn beacon_port(&self) -> u16 {
        self.beacon_port
    }
}

// Defaults
fn default_true() -> bool{
    true
}

fn default_false() -> bool{
    false
}

fn default_listen_addr() -> String {
    "0.0.0.0:1930".into()
}

fn default_pub_addr() -> String {
    "0.0.0.0:1930".into()
}

fn default_log_path() -> String {
    "spider.log".into()
}

fn default_state_data_path() -> String {
    "state.dat".into()
}

fn default_beacon_port() -> u16 {
    1930u16
}

fn default_veilid_enabled() -> bool {
    true
}

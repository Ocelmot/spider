use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::SpiderId2048;

#[derive(Debug, Serialize, Deserialize)]
pub struct Keyfile {
    pub id: SpiderId2048,
    pub permission_code: Option<String>,
}

impl Keyfile {
    pub fn new(id: SpiderId2048, permission_code: Option<String>) -> Self {
        Self {
            id,
            permission_code,
        }
    }

    pub async fn write_new(path: PathBuf, id: SpiderId2048, permission_code: Option<String>) {
        let keyfile = Self {
            id,
            permission_code,
        };
        keyfile.write_to_file(path).await;
    }

    pub async fn write_to_file<P>(&self, path: P) where
    P: AsRef<Path>,{
        let data = serde_json::to_string(&self).unwrap();
        tokio::fs::write(path, data).await;
    }

    pub async fn read_from_file<P>(path: P) -> Option<Self>
    where
        P: AsRef<Path>,
    {
        match tokio::fs::read_to_string(path).await {
            Ok(str) => match serde_json::from_str(&str) {
                Ok(keyfile) => keyfile,
                Err(_) => None,
            },
            Err(_) => None,
        }
    }
}

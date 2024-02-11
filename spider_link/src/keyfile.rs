use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::SpiderId2048;


/// A Keyfile is used to transfer connection parameters from the base to a
/// peripheral to establish a connection to the base. It only needs to be
/// used once as these parameters are saved in the peripheral's state.
/// The connection parameters are the public key used to establish the
/// encrypted connection to the base, and a permission code to allow the
/// peripheral to automatically be accepted as an approved connection.
#[derive(Debug, Serialize, Deserialize)]
pub struct Keyfile {
    /// The Keyfile's id
    pub id: SpiderId2048,
    /// The Keyfile's permission code
    pub permission_code: Option<String>,
}

impl Keyfile {
    /// Creates a new Keyfile from a public id, and a permission code
    pub fn new(id: SpiderId2048, permission_code: Option<String>) -> Self {
        Self {
            id,
            permission_code,
        }
    }

    /// Writes a Keyfile to the given path using the constituant
    /// parts of the keyfile.
    pub async fn write_new(path: PathBuf, id: SpiderId2048, permission_code: Option<String>) {
        let keyfile = Self {
            id,
            permission_code,
        };
        keyfile.write_to_file(path).await;
    }

    /// Writes a keyfile out to a path
    pub async fn write_to_file<P>(&self, path: P) where
    P: AsRef<Path>,{
        let data = serde_json::to_string(&self).unwrap();
        tokio::fs::write(path, data).await;
    }

    /// Reads a keyfile from a path, returns None if there was an error.
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

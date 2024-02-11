use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::SpiderClientState;
use spider_link::{Relation, Keyfile, Role};

use self::processor::SpiderClientProcessor;

mod channel;
pub mod processor;
pub use channel::ClientChannel;

mod message;
pub use message::{ClientControl, ClientResponse};

/// SpiderClientBuilder contains a set of settings that can be loaded
/// from a file, modified, saved back to a file, or used to connect
/// to a Spider base.
#[derive(Debug, Clone)]
pub struct SpiderClientBuilder {
    state_path: Option<PathBuf>,
    state: SpiderClientState,
}

impl SpiderClientBuilder {
    /// Create a new SpiderClientBuilder with no path and default state
    pub fn new() -> Self {
        SpiderClientBuilder {
            state_path: None,
            state: SpiderClientState::new(),
        }
    }

    /// Use the given path as the path for this builder, and load the state from that file.
    pub fn load(path: &Path) -> Self {
        let state = SpiderClientState::from_file(path);
        Self {
            state_path: Some(path.to_path_buf()),
            state,
        }
    }

    /// Use the given path as the path for this builder, and load the state from that file
    /// if it exists. If the file does not exist, create a default builder and pass it to the
    /// callback to set the initial state. The state will be saved afterward.
    pub fn load_or_set<F: FnOnce(&mut SpiderClientBuilder)>(path: &Path, func: F) -> Self {
        match fs::read_to_string(&path).and_then(|data| {
            SpiderClientState::from_string(data).ok_or(io::ErrorKind::InvalidData.into())
        }) {
            Ok(state) => Self {
                state_path: Some(path.to_owned()),
                state,
            },
            Err(_) => {
                let mut client = Self {
                    state_path: Some(path.to_owned()),
                    state: SpiderClientState::new(),
                };
                func(&mut client);
                client.save();
                client
            }
        }
    }

    /// Write the current state out to the path used to create it.
    pub fn save(&self) {
        if let Some(path) = &self.state_path {
            self.state.to_file(path)
        }
    }

    /// Use the configuration in the state to create a new connection to a base.
    /// Set enable_recv to true to enable recv for the resulting channel from the start.
    pub fn start(self, enable_recv: bool) -> ClientChannel {
        let (channel, _) = SpiderClientProcessor::start(self.state_path, self.state, enable_recv);
        channel
    }

    // State modification functions
    /// Set or update the path where the state will be saved.
    pub fn set_state_path(&mut self, path: &PathBuf) {
        self.state_path = Some(path.to_path_buf())
    }

    /// Get the Relation for the peripheral side of the connection.
    pub fn self_relation(&self) -> &Relation {
        &self.state.self_relation.relation
    }

    /// Returns true if the state is paired with some base.
    pub fn has_host_relation(&self) -> bool {
        self.state.host_relation.is_some()
    }

    /// Set the Relation of the base that this peripheral should be paired to.
    pub fn set_host_relation(&mut self, relation: Relation) {
        self.state.host_relation = Some(relation);
    }

    /// Unpair this peripheral from any base it is paired with.
    pub fn clear_host_relation(&mut self) {
        self.state.host_relation = None;
    }

    /// If there is a permission code that will allow this peripheral to be
    /// automatically approved, add that code to the state.
    pub fn set_permission_code(&mut self, code: Option<String>) {
        self.state.permission_code = code;
    }

    // Connection strategies
    /// Enable or disable the use of the last address connection strategy.
    /// Enabled by default.
    /// The last address connection strategy saves the last known address
    /// to use when reconnecting. This should work very well when reconnecting
    /// to the same network, but will fail whenever
    /// first connecting to a new network.
    pub fn enable_last_addr(&mut self, set: bool) {
        self.state.last_addr_enable = set;
    }
    /// Manually override the last known address
    pub fn set_last_addr(&mut self, set: Option<String>) {
        self.state.set_last_addr(set);
    }

    // Beacon
    /// Enable or disable the use of the beacon connection strategy.
    /// Enabled by default
    /// The beacon connection strategy will broadcast a probe that
    /// bases on the same network should respond to. Can be used to find
    /// bases that are not yet paired.
    pub fn enable_beacon(&mut self, set: bool) {
        self.state.beacon_enable = set;
    }

    // Chord
    /// Enable or disable the use of the chord connection strategy.
    /// Enabled by default.
    /// The chord connection strategy connects to the chord, and uses it
    /// to lookup the public address of the base.
    /// The chord connection strategy only works after a base has been paired.
    pub fn enable_chord(&mut self, set: bool) {
        self.state.chord_enable = set;
    }
    /// Manually set the list of recent chord node addresses.
    pub fn set_chord_addrs(&mut self, set: Vec<String>) {
        self.state.chord_addrs = set;
    }

    // Fixed addresses
    /// Enable or disable the use of the fixed address connection strategy.
    /// Disabled by default.
    /// The fixed address connection strategy maintains an unchanging list
    /// of addresses to try to connect to a base.
    /// Useful for static addresses, or if the base is always on localhost.
    pub fn enable_fixed_addrs(&mut self, set: bool) {
        self.state.fixed_addr_enable = set;
    }
    /// Set the list of fixed addresses
    pub fn set_fixed_addrs(&mut self, addrs: Vec<String>) {
        self.state.fixed_addrs = addrs;
    }

    // Other Operations
    /// Load the base's key and optional permission code from the given file
    /// into the state configuration.
    /// If the file is missing, no error is reported.
    pub async fn try_use_keyfile<P>(&mut self, path: P)
    where
        P: AsRef<Path>,
    {
        let keyfile = Keyfile::read_from_file(path).await;
        if let Some(keyfile) = keyfile {
            let other_relation = Relation {id: keyfile.id, role: Role::Peer};
            self.set_host_relation(other_relation);
            self.set_permission_code(keyfile.permission_code);
            self.save();
        }
    }
}

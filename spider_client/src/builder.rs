use std::{
    fs, num::NonZeroUsize, path::{Path, PathBuf}
};

use spider_link::{Keyfile, Relation, Role, SelfRelation, link_set::links::Address};

use crate::{
    error::{ClientResult, ErrorKind, ProblemWrap},
    processor::SpiderClientProcessor,
    state::SpiderClientState,
    ClientChannel,
};

/// SpiderClientBuilder contains settings that can be loaded from a file,
/// modified, saved back to a file, or used to connect to a Spider base.
#[derive(Debug, Clone)]
pub struct SpiderClientBuilder {
    state_path: Option<PathBuf>,
    state: SpiderClientState,
}

impl SpiderClientBuilder {
    /// Create a new SpiderClientBuilder with the given path and state
    pub(crate) fn new<P>(path: Option<P>, state: SpiderClientState) -> Self
    where
        P: Into<PathBuf>,
    {
        SpiderClientBuilder {
            state_path: path.map(|p| p.into()),
            state,
        }
    }

    /// Create a new SpiderClientBuilder where the state is stored at the given
    /// path, but the SelfRelation is given by the parameter
    pub fn new_with_self_relation<P>(path: P, self_rel: SelfRelation) -> Self
    where
        P: Into<Option<PathBuf>>,
    {
        let state = SpiderClientState::new(self_rel);
        Self {
            state_path: path.into(),
            state,
        }
    }

    /// Use the given path as the path for this builder, and load the state from
    /// that file.
    pub async fn load<P>(path: P) -> ClientResult<Self>
    where
        P: Into<PathBuf>,
    {
        let path = path.into();
        let state = SpiderClientState::from_file(&path).await?;
        Ok(Self {
            state_path: Some(path),
            state,
        })
    }

    /// Use the given path as the path for this builder, and load the state from
    /// that file if it exists. If the file does not exist, create a default
    /// builder and pass it to the callback to set the initial state. The state
    /// will be saved afterward.
    pub async fn load_or_set<F>(path: &Path, func: F) -> ClientResult<Self>
    where
        F: FnOnce(&mut SpiderClientBuilder),
    {
        let state = fs::read_to_string(&path)
            .wrap_problem(ErrorKind::IO)
            .and_then(|contents| SpiderClientState::from_string(contents));

        match state {
            Ok(state) => Ok(Self {
                state_path: Some(path.to_owned()),
                state,
            }),
            Err(_) => {
                let mut client = Self {
                    state_path: Some(path.to_owned()),
                    state: SpiderClientState::default(),
                };
                func(&mut client);
                client.save().await?;
                Ok(client)
            }
        }
    }

    /// Write the current state out to the path used to create it.
    pub async fn save(&self) -> ClientResult {
        if let Some(path) = &self.state_path {
            self.state.to_file(path).await
        } else {
            Ok(())
        }
    }

    /// Use the configuration in the state to create a new connection to a base.
    /// Set enable_recv to true to enable recv for the resulting channel from the start.
    pub async fn start(self, enable_recv: bool) -> ClientResult<ClientChannel> {
        let (channel, _) =
            SpiderClientProcessor::start_processor(self.state_path, self.state, enable_recv).await?;
        Ok(channel)
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
    pub fn clear_host_relation(&mut self) -> Option<Relation>{
        self.state.host_relation.take()
    }

    /// If there is a permission code that will allow this peripheral to be
    /// automatically approved, add that code to the state.
    pub fn set_permission_code(&mut self, code: String) {
        self.state.permission_code = Some(code);
    }

    /// If there is a permission code that will allow this peripheral to be
    /// automatically approved, add that code to the state.
    pub fn clear_permission_code(&mut self) {
        self.state.permission_code = None;
    }

    /// Set whether the client should automatically try to reestablish the
    /// connection.
    pub fn auto_reconnect(&mut self, auto_reconnect: bool) {
        self.state.auto_reconnect = auto_reconnect;
    }

    /// Enable a transport method for this client
    pub fn enable_transport(&mut self, scheme: String) {
        self.state.transports.insert(scheme);
    }

    /// Disable a transport method for this client
    pub fn disable_transport(&mut self, scheme: &String) {
        self.state.transports.remove(scheme);
    }

    // Base Addrs
    /// Set whether the client should use addresses provided by the base when
    /// establishing a connection
    pub fn base_addrs_enable(&mut self, enable: bool) {
        self.state.base_addrs_enable = enable;
    }

    /// Resizes the LRU set of addresses maintained from the base.
    /// 
    /// Setting this to zero will clear the set, and disable the base addrs
    /// function.
    pub fn resize_base_addr_set(&mut self, cap: usize) {
        if let Some(cap) = NonZeroUsize::new(cap) {
            self.state.base_addrs.resize(cap);
        }else {
            self.state.base_addrs.clear();
            self.state.base_addrs_enable = false;
        }
    }

    /// Manually insert an address into the LRU set of addresses used when
    /// establishing a connection. This may evict another address.
    pub fn add_base_addr(&mut self, addr: Address) {
        self.state.base_addrs.push(addr, ());
    }

    /// Clear all addrs received from the base for reconnection.
    pub fn clear_base_addrs(&mut self) {
        self.state.base_addrs.clear();
    }

    // Discovery
    /// Enable or disable the use of whichever discovery strategy is available
    /// on the platform.
    /// 
    /// Enabled by default
    /// 
    /// The beacon connection strategy will broadcast a probe that bases on the
    /// same network should respond to. Can be used to find bases that are not
    /// yet paired. On platforms that disallow UDP, the mdns strategy will be
    /// used instead.
    pub fn enable_discovery(&mut self, set: bool) {
        self.state.discovery_enable = set;
    }

    /// Returns the port the beacon will use when it tries to find the base. Not
    /// applicable to mdns.
    pub fn beacon_port(&self) -> u16 {
        self.state.beacon_port
    }

    // Veilid
    /// Enables the use of veilid to establish a connection. This provides the
    /// name of the client to veilid. It should be unique to the application
    /// requesting the connections. It must also be a valid filename.
    pub fn enable_veilid<S: Into<String>>(&mut self, client_name: S) {
        self.state.veilid_enable = Some(client_name.into());
    }

    /// Disables the use of veilid to establish a connection
    pub fn disable_veilid(&mut self) {
        self.state.veilid_enable = None;
    }

    /// Set the path to where the veilid subsystem will store its state.
    /// 
    /// If assigned to None, it will use the default of ./.veilid/
    pub fn veilid_root(&mut self, root: Option<String>) {
        self.state.veilid_root = root;
    }

    /// Sets the stored dht key by which veilid will listen for connections.
    /// This is usually done automatically, there should be no need to generate
    /// a key manually.
    // pub fn set_veilid_dht(&mut self, dht_key: DHTRecordDescriptor) {
    //     self.state.veilid_own_dht = Some(dht_key);
    // }

    /// Clears the stored dht key by which veilid will listen for connections.
    pub fn clear_veilid_dht(&mut self) {
        self.state.veilid_own_dht = None;
    }

    // Fixed addresses
    /// Enable or disable the use of the fixed address connection strategy.
    /// Disabled by default.
    /// The fixed address connection strategy maintains an unchanging list
    /// of addresses to try to connect to a base.
    /// Useful for static addresses, or if the base is always on localhost.
    pub fn enable_fixed_addrs(&mut self, enable: bool) {
        self.state.fixed_addr_enable = enable;
    }

    /// Add an address to the list of fixed addresses to try when connection.
    pub fn add_fixed_addr(&mut self, addr: Address) {
        self.state.fixed_addrs.push(addr);
    }

    /// Set the list of fixed addresses
    pub fn set_fixed_addrs(&mut self, addrs: Vec<Address>) {
        self.state.fixed_addrs = addrs;
    }

    /// Clear the list of fixed addresses
    pub fn clear_fixed_addrs(&mut self) {
        self.state.fixed_addrs.clear();
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
            let other_relation = Relation {
                id: keyfile.id,
                role: Role::Peer,
            };
            self.set_host_relation(other_relation);
            if let Some(code) = keyfile.permission_code {
                self.set_permission_code(code);
            }
            let _ = self.save().await;
        }
    }
}

impl Default for SpiderClientBuilder {
    fn default() -> Self {
        Self {
            state_path: Default::default(),
            state: Default::default(),
        }
    }
}

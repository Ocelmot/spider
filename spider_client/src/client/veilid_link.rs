use std::{
    collections::HashSet, mem, path::{Path, PathBuf}, sync::Arc
};

use log::{debug, error, info};
use spider_link::{
    message::{FrameManager, Message, VeilidFrame, VeilidMessage},
    Relation, SelfRelation,
};
use tokio::{
    select, spawn,
    sync::{
        mpsc::{channel, Receiver, Sender, UnboundedReceiver, UnboundedSender}, Mutex, OnceCell, RwLock
    },
    task::JoinHandle,
};
use veilid_core::{
    DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, KeyPair, RouteId, RoutingContext, TypedKey,
    VeilidAPI, VeilidAPIError, VeilidAPIResult, VeilidConfigBlockStore, VeilidConfigInner,
    VeilidConfigProtectedStore, VeilidConfigTableStore, VeilidUpdate,
};

static CONFIG_ROOT: Mutex<Option<PathBuf>> = Mutex::const_new(None);
static API_CHANNELS: RwLock<Vec<UnboundedSender<VeilidUpdate>>> = RwLock::const_new(Vec::new());
static API: OnceCell<VeilidAPI> = OnceCell::const_new();

pub struct VeilidLink {
    link_tx: Sender<Message>,
    link_rx: Receiver<Message>,
    handle: JoinHandle<()>,
}

impl VeilidLink {
    pub async fn get_incoming_dht() -> Option<DHTRecordDescriptor> {
        info!("Getting incoming dht, outer");
        VeilidProcessor::get_incoming_dht().await
    }

    pub async fn new(
        us: SelfRelation,
        other_relation: Relation,
        other_dht_key: TypedKey,
    ) -> Option<Self> {
        let (link_tx, processor_rx) = channel(50);
        let (processor_tx, link_rx) = channel(50);

        let processor = VeilidProcessor::new(
            us,
            other_relation,
            processor_tx,
            processor_rx,
            other_dht_key,
        )
        .await?;

        let handle = spawn(async move {
            processor.run().await;
        });

        Some(Self {
            link_tx,
            link_rx,
            handle,
        })
    }

    // start function

    pub async fn send(&self, msg: Message) -> Option<()> {
        self.link_tx.send(msg).await.ok()
    }

    pub async fn recv(&mut self) -> Option<Message> {
        self.link_rx.recv().await
    }
}

struct VeilidProcessor {
    // Spider details
    us: SelfRelation,
    other_relation: Relation,

    // transmission channels
    processor_tx: Sender<Message>,
    processor_rx: Receiver<Message>,
    veilid_rx: UnboundedReceiver<VeilidUpdate>,

    // Veilid components
    api: VeilidAPI,
    routing: RoutingContext,

    // Outgoing (Sending to a node)
    other_dht_key: TypedKey,
    outgoing_route: Option<RouteId>,
    last_known_blob: Vec<u8>,
    message_buffer: Vec<Message>,

    // Incoming, recieving from a node
    incoming_routes: HashSet<RouteId>,
    frame_manager: FrameManager,
}

impl VeilidProcessor {
    async fn get_incoming_dht() -> Option<DHTRecordDescriptor> {
        info!("Getting Incoming DHT, Inner");
        let api = get_veilid_api(None).await.ok()?;
        let store = api.table_store().expect("api should be valid");

        let table = store.open("incoming_dht", 1).await.unwrap();
        match table.load_json(0, b"own_dht").await.ok()? {
            Some(dht_entry) => {
                info!("Entry found in table");
                Some(dht_entry)
            }
            None => {
                info!("Nothing stord in table");
                // create and store
                let routing = api.routing_context().expect("routing failed");
                let schema = DHTSchema::DFLT(DHTSchemaDFLT::new(1).expect("schema should create"));
                // let kind = self.get_own_keypair().await.unwrap().kind;
                let dht_descriptor = routing.create_dht_record(schema, None).await.ok()?;
                info!("Storing dht_descriptor");
                table.store_json(0, b"own_dht", &dht_descriptor).await;
                info!("Returning dht_descriptor");
                Some(dht_descriptor)
            }
        }
    }

    async fn new(
        us: SelfRelation,
        other_relation: Relation,
        processor_tx: Sender<Message>,
        processor_rx: Receiver<Message>,
        other_dht_key: TypedKey,
    ) -> Option<Self> {
        let (veilid_tx, veilid_rx) = tokio::sync::mpsc::unbounded_channel();
        let api = get_veilid_api(Some(veilid_tx)).await;

        let api = match api {
            Ok(api) => api,
            Err(e) => {
                error!("VeilidProcessor encountered error: {}", e);
                return None;
            }
        };

        let _ = api.attach().await;

        // Check that we can get a route id from the other_dht_key
        let routing = api.routing_context().ok()?;
        debug!("Opening dht record...");
        routing.open_dht_record(other_dht_key, None).await.ok()?;

        debug!("Getting dht value from paired key...");
        let last_known_blob = routing.get_dht_value(other_dht_key, 0, true).await.ok()??;
        let last_known_blob = last_known_blob.data().to_vec();

        debug!("Getting route from imported blob...");
        let outgoing_route = api
            .import_remote_private_route(last_known_blob.clone())
            .ok()?;
        debug!("Got all components for initialization");
        // Setup incoming route
        let incoming_routes = HashSet::new();

        let mut new_self = Self {
            // Spider Details
            us,
            other_relation: other_relation.clone(),

            // Transmission Channels
            processor_tx,
            processor_rx,
            veilid_rx,

            // Veilid Components
            api,
            routing,

            // Outgoing
            other_dht_key,
            outgoing_route: Some(outgoing_route),
            last_known_blob,
            message_buffer: Vec::new(),

            // Incoming
            incoming_routes,
            frame_manager: FrameManager::new(other_relation),
        };

        new_self.install_route().await;

        Some(new_self)
    }

    async fn run(mut self) {
        loop {
            select! {
                // recv from link
                    // send to veilid
                msg = self.processor_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            match self.send_via_veilid(msg).await {
                                Ok(_) => {},
                                Err(_) => {return;}, // disconnected
                            }                            
                        },
                        None => {
                            // The control link has closed, shutdown
                            break;
                        },
                    }
                }
                // recv from veilid
                    // send to link
                msg = self.veilid_rx.recv() => {
                    match msg {
                        Some(msg) => {
                            debug!("Recieved a message from Veilid network");
                            self.recv_veilid_msg(msg).await;
                        },
                        None => {
                            // Connection from veilid failed, it must be closed
                            break;
                        },
                    }
                }
            }
        }
    }

    async fn send_via_veilid(&mut self, msg: Message) -> VeilidAPIResult<()> {
        debug!("sending message via veilid");
        if !self.message_buffer.is_empty() {
            debug!("buffer has message, appending...");
            // if there are are messages in the buffer, new message should come behind those ones.
            self.message_buffer.push(msg);
            return Ok(());
        }
        // try to send message via the api        

        let route_id = if let Some(route_id) = self.outgoing_route {
            debug!("Using stored route_id");
            route_id
        } else {
            debug!("Using route id from dht");
            // get new blob
            let new_blob = self
                .routing
                .get_dht_value(self.other_dht_key, 0, true)
                .await
                .ok()
                .flatten()
                .unwrap();
            let new_blob = new_blob.data().to_vec();
            debug!("Got blob = {:?}", new_blob);
            self.last_known_blob = new_blob.clone();
            // get new route_id
            let new_route_id = self.api.import_remote_private_route(new_blob).unwrap();
            debug!("new_route_id: {:?}", new_route_id);
            self.outgoing_route = Some(new_route_id.clone());
            new_route_id
        };

        let frames = VeilidFrame::new_wrapped_msg(&self.us, msg.clone());
        for frame in frames {
            println!("Sending frame! print");
            debug!("Sending frame! debug");
            tracing::debug!("Sending frame! tracing");
            tracing::debug!("Sending frame {} in sequence of {}", frame.index(), frame.count());
            let data = serde_json::to_vec(&frame).unwrap();
            let res = self.routing
                .app_message(veilid_core::Target::PrivateRoute(route_id), data)
                .await;
            debug!("app_message result: {:?}", res);
            if let VeilidAPIResult::Err(VeilidAPIError::TryAgain { .. }) = res {
                // add to the message queue to send later
                debug!("Send failed at frame index: {} of {}, adding to buffer.", frame.index(), frame.count());
                self.message_buffer.push(msg);
                return Ok(());
            }
            if res.is_err() {
                return res;
            }
        }
        Ok(())
    }

    async fn recv_veilid_msg(&mut self, msg: VeilidUpdate) {
        match msg {
            VeilidUpdate::AppMessage(msg) => {
                // Ignore messages sent without a private route
                if msg.route_id().is_some() {
                    // Ignore messages that do not deserialize properly
                    if let Ok(veilid_frame) = serde_json::from_slice::<VeilidFrame>(msg.message()) {
                        match self.frame_manager.add_frame(veilid_frame) {
                            Some(message) => {
                                debug!("Got message from frame");
                                self.handle_veilid_msg(message).await;
                            },
                            None => {
                                debug!("Frame was part of sequence");
                            },
                        }
                    } else {
                        info!("failed to deserialize Veilid Message");
                    }
                }
            }
            VeilidUpdate::Attachment(attachment) => {
                if attachment.public_internet_ready {
                    debug!("Attachemnt indicates internet ready");
                    // try to generate a new route when connected to internet
                    // This function will not install a route if there is an existing one.
                    debug!("installing route");
                    self.install_route().await;
                    self.outgoing_route = None; // Clear to cause a re-request of the route
                    // flush out the buffer, sending the saved messages
                    for message in mem::take(&mut self.message_buffer) {
                        debug!("Sending buffered message");
                        self.send_via_veilid(message).await;
                    }
                }
            }
            VeilidUpdate::RouteChange(changes) => {
                // replace incoming route in dht
                for route_id in changes.dead_routes {
                    if self.incoming_routes.remove(&route_id) {
                        self.install_route().await;
                    }
                }

                // replace outgoing route
                for route_id in changes.dead_remote_routes {
                    if let Some(outgoing_route) = self.outgoing_route {
                        if outgoing_route == route_id {
                            // Get new route blob
                            // compare to stored blob.
                            // if different, import that blob
                            debug!("Reestablishing routeid");
                            debug!("Opening dht record...");
                            self.routing.open_dht_record(self.other_dht_key, None).await;

                            debug!("Getting dht value from paired key...");
                            let last_known_blob = self.routing.get_dht_value(self.other_dht_key, 0, true).await.ok().flatten().unwrap();
                            let last_known_blob = last_known_blob.data().to_vec();

                            debug!("Getting route from imported blob...");
                            self.outgoing_route = self.api
                                .import_remote_private_route(last_known_blob.clone())
                                .ok();
                            // TODO: if this fails, there is no way to recover since the route expiration will not happen again
                        }
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_veilid_msg(&mut self, msg: VeilidMessage) {
        info!("Handling VeilidMessage");

        match msg {
            // A generated invite code was used by some node to establish a connection.
            // This needs to be verified first
            VeilidMessage::CompleteInvite { .. } => {
                info!("completing invite (no action)");
            }
            VeilidMessage::Message(msg) => {
                info!("Recieved message");
                self.processor_tx.send(msg).await;
            }
        }
    }

    async fn install_route(&mut self) -> Option<()> {
        if !self.incoming_routes.is_empty() {
            // there exists a valid route, no reason to create another
            return Some(());
        }
        // create a new route
        let (route_id, route_blob) = self.api.new_private_route().await.ok()?;
        // store the blob in the dht
        let dht_descriptor = Self::get_incoming_dht().await?;
        let writer = Some(KeyPair::new(
            dht_descriptor.owner().clone(),
            *dht_descriptor.owner_secret().unwrap(),
        ));
        let dht_key = dht_descriptor.key().clone();
        info!("our dht_key: {:?}", dht_key);

        // network write
        self.routing.open_dht_record(dht_key, writer).await.ok()?;
        self.routing
            .set_dht_value(dht_key, 0, route_blob, None)
            .await
            .ok()?;

        // store the id in the set
        self.incoming_routes.insert(route_id);
        Some(())
    }
}

async fn get_veilid_api(
    sender: Option<UnboundedSender<VeilidUpdate>>,
) -> VeilidAPIResult<VeilidAPI> {
    // Insert the sender into senders
    if let Some(sender) = sender {
        let mut senders = API_CHANNELS.write().await;
        senders.push(sender);
    }

    // get or initialize the API instance
    let api = API
        .get_or_try_init(|| {
            async {
                info!("Initializing Veilid API");
                let (inner_tx, mut inner_rx) = tokio::sync::mpsc::unbounded_channel();
                let default_path = PathBuf::from("./.veilid");
                let path_binding = CONFIG_ROOT
                    .lock().await;
                let config_path = path_binding.as_ref()
                    .unwrap_or(&default_path);

                let api = veilid_core::api_startup_config(
                    Arc::new(move |arg| {
                        // Get the senders, then send for each sender
                        let _ = inner_tx.send(arg);
                    }),
                    veilid_config(&config_path),
                )
                .await?;

                tokio::spawn(async move {
                    let mut empty_indices = HashSet::new();
                    while let Some(update) = inner_rx.recv().await {
                        let channels = API_CHANNELS.read().await;
                        for (index, channel) in channels.iter().enumerate() {
                            if channel.send(update.clone()).is_err() {
                                empty_indices.insert(index);
                            }
                        }
                        drop(channels);
                        if !empty_indices.is_empty() {
                            // some channel failed, needs to be cleaned out
                            // This is a separate operation to avoid acquiring a
                            // write lock on the RW lock
                            let mut channels = API_CHANNELS.write().await;
                            let mut index = 0;
                            channels.retain(|_| {
                                let is_dead = empty_indices.contains(&index);
                                index += 1;
                                is_dead
                            });
                            drop(channels);

                            empty_indices.clear();
                        }
                    }
                });

                VeilidAPIResult::Ok(api)
            }
        })
        .await;
    api.cloned()
}

/// Sets the root for Veilid configuration paths.
/// If this is still None by the time the Veilid instance is started, it will
/// not take effect.
pub async fn set_veilid_path_root(path: Option<PathBuf>) {
    let mut config_root_global = CONFIG_ROOT.lock().await;
    *config_root_global = path;
}

fn veilid_config(config_root: &Path) -> VeilidConfigInner {
    VeilidConfigInner {
        program_name: "Spider GUI".into(),
        namespace: "spider_gui".into(),
        protected_store: VeilidConfigProtectedStore {
            directory: {
                let path = config_root.join("protected_store");
                path.to_str().unwrap().to_string()
            },
            ..Default::default()
        },
        block_store: VeilidConfigBlockStore {
            directory: {
                let path = config_root.join("block_store");
                path.to_str().unwrap().to_string()
            },
            ..Default::default()
        },
        table_store: VeilidConfigTableStore {
            directory: {
                let path = config_root.join("block_store");
                path.to_str().unwrap().to_string()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

use std::{collections::HashMap, sync::Arc, time::Duration};

use bimap::BiHashMap;
use log::{debug, error, info};
use rand::Rng;
use spider_link::{
    message::{
        DirectoryEntry, FrameManager, Invite, Message, RouterMessage, UiInput, VeilidFrame,
        VeilidInvite, VeilidMessage,
    },
    Relation,
};
use tokio::{
    select,
    sync::mpsc::{channel, error::SendError, Receiver, Sender, UnboundedReceiver},
    task::{JoinError, JoinHandle},
};
use veilid_core::{
    Crypto, CryptoTyped, DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, FromStr, KeyPair, RouteId,
    Target, TypedKey, VeilidAPI, VeilidConfigBlockStore, VeilidConfigInner,
    VeilidConfigProtectedStore, VeilidConfigTableStore, VeilidUpdate, CRYPTO_KIND_VLD0,
};

use crate::processor::{
    link::ProcessorLink, message::ProcessorMessage, router::RouterProcessorMessage,
    ui::UiProcessorMessage,
};

fn veilid_config() -> VeilidConfigInner {
    VeilidConfigInner {
        program_name: "Spider".into(),
        namespace: "spider".into(),
        protected_store: VeilidConfigProtectedStore {
            directory: "./.veilid/block_store".into(),
            ..Default::default()
        },
        block_store: VeilidConfigBlockStore {
            directory: "./.veilid/block_store".into(),
            ..Default::default()
        },
        table_store: VeilidConfigTableStore {
            directory: "./.veilid/table_store".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[derive(Debug)]
pub enum VeilidProcessorMessage {
    /// Process a message to be sent via the Veilid network.
    RouteMessage(Relation, Message),

    /// Enables Veilid to be used for this relation using this key
    VeilidEnabled(Relation, TypedKey),

    /// Create a new invite to be sent to another user.
    /// The invite will be stored in the settings menu.
    /// The parameter is an optional relation that will also recieve a message
    /// about the newly generated invite.
    GenerateInvite(Option<Relation>),

    /// Accept an invite send from another user, via this user.
    AcceptInvite(VeilidInvite),
    /// Revoke an invitation using the string id.
    /// In the case of Veilid, the string is a RouteId
    /// and should be parsed that way.
    RevokeInvite(String),

    /// Process an Upkeep tick
    Upkeep,
}

pub(crate) struct VeilidProcessor {
    sender: Sender<VeilidProcessorMessage>,
    handle: JoinHandle<()>,
}

impl VeilidProcessor {
    pub async fn new(pl: ProcessorLink) -> Option<Self> {
        let (veilid_sender, veilid_receiver) = channel(50);
        let processor = VeilidProcessorState::new(pl, veilid_receiver).await;
        match processor {
            Some(processor) => {
                let handle = processor.start();
                Some(Self {
                    sender: veilid_sender,
                    handle,
                })
            }
            None => None,
        }
    }

    pub async fn send(
        &self,
        msg: VeilidProcessorMessage,
    ) -> Result<(), SendError<VeilidProcessorMessage>> {
        self.sender.send(msg).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

pub(crate) struct VeilidProcessorState {
    pl: ProcessorLink,
    receiver: Receiver<VeilidProcessorMessage>,
    api: VeilidAPI,
    veilid_rx: UnboundedReceiver<VeilidUpdate>,
    /// The blob that is posted to our DHT entry
    incoming_route: Option<RouteId>,
    outgoing_routes: BiHashMap<Relation, RouteId>,
    frame_managers: HashMap<Relation, FrameManager>,
}

impl VeilidProcessorState {
    async fn new(pl: ProcessorLink, receiver: Receiver<VeilidProcessorMessage>) -> Option<Self> {
        let (veilid_tx, veilid_rx) = tokio::sync::mpsc::unbounded_channel();
        let api = veilid_core::api_startup_config(
            Arc::new(move |arg| {
                let _ = veilid_tx.send(arg);
            }),
            veilid_config(),
        )
        .await;
        let api = match api {
            Ok(api) => api,
            Err(e) => {
                error!("VeilidProcessor encountered error: {}", e);
                return None;
            }
        };

        Some(Self {
            pl,
            receiver,
            api,
            veilid_rx,
            incoming_route: None,
            outgoing_routes: BiHashMap::new(),
            // pending_messages: HashMap::new(),
            frame_managers: HashMap::new(),
        })
    }

    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            self.init().await;

            // Process messages
            loop {
                select! {
                    msg = self.veilid_rx.recv() => {
                        match msg {
                            Some(msg) => {
                                self.recv_veilid_msg(msg).await;
                            },
                            None => {
                                // Connection from veilid failed, it must be closed
                                break;
                            },
                        }
                    }

                    msg = self.receiver.recv() => {
                        match msg {
                            Some(msg) => {
                                self.recv_control_msg(msg).await;
                            },
                            None => {
                                // The control link has closed, shutdown
                                break;
                            },
                        }
                    }
                };
            }
            self.api.detach().await;
        });
        handle
    }

    async fn init(&mut self) {
        let store = self.api.protected_store().unwrap();
        // Initialize this node's keys if missing
        let key = self.get_own_keypair().await;
        let keypair = match key {
            Some(key) => key,
            None => {
                // Generate a new key
                let keypair = Crypto::generate_keypair(CRYPTO_KIND_VLD0)
                    .expect("should be able to generate keys");
                store
                    .save_user_secret_string("own_keys", keypair.to_string())
                    .await
                    .unwrap();
                keypair
            }
        };

        // let store = self.api.table_store().unwrap();
        // store.delete("incoming_dht").await;

        let dht_key = self.load_incoming_dht().await;
        if dht_key.is_none() {
            info!("DHT Entry not fround, Creating new DHT");
            let routing = self
                .api
                .routing_context()
                .expect("api should be initialized");

            let schema = DHTSchema::DFLT(DHTSchemaDFLT::new(1).expect("schema should create"));
            let kind = self.get_own_keypair().await.unwrap().kind;
            let dht_descriptor = routing
                .create_dht_record(schema, Some(kind))
                .await
                .expect("should be able to make a DHT key");
            info!("New DHT entry {:?}", dht_descriptor);
            self.store_incoming_dht(dht_descriptor).await;
        }

        // Add setting item to accept invites from others
        let msg = UiProcessorMessage::SetSetting {
            header: "Pending Connections".into(),
            title: "Accept Veilid Invite".into(),
            inputs: vec![("textentry".into(), "Enter Invite".into())],
            cb: |e| {
                if let UiInput::Text(invite_code) = e.input() {
                    let invite = Invite::from_base64(invite_code)?;
                    let msg = RouterProcessorMessage::AcceptInvite(invite);
                    let msg = ProcessorMessage::RouterMessage(msg);
                    Some(msg)
                } else {
                    None
                }
            },
            data: String::new(),
        };
        self.pl.send_ui(msg).await;

        // read in pending invites for the settings menu
        let invites = self.load_pending_invites().await;
        for invite in invites {
            self.add_invite_to_settings(*invite.dht_key(), invite.code())
                .await;
        }

        match self.api.attach().await {
            Ok(_) => info!("Successfully attached to Veilid network"),
            Err(e) => info!("Failed to attach to Veilid network with error: {:?}", e),
        }

        // Generate new Route
        self.install_route().await;
    }

    async fn recv_veilid_msg(&mut self, veilid_update: VeilidUpdate) {
        match veilid_update {
            VeilidUpdate::AppMessage(msg) => {
                info!("Recieved app_message: {:?}", msg);

                // Ignore messages sent without a private route
                if msg.route_id().is_some() {
                    // Ignore messages that do not deserialize properly
                    if let Ok(veilid_frame) = serde_json::from_slice::<VeilidFrame>(msg.message()) {
                        // Now, we add it to the frame manager for this relation
                        // if the addition returns a VeilidMessage, handle that message.
                        let rel = veilid_frame.rel().clone();
                        let entry = self.frame_managers.entry(rel.clone());
                        let manager = entry.or_insert_with(|| FrameManager::new(rel.clone()));
                        if let Some(msg) = manager.add_frame(veilid_frame) {
                            debug!("Frame completed, processing...");
                            self.handle_veilid_message(rel, msg).await;
                        } else {
                            debug!("Frame did not complete a sequence");
                        }
                    } else {
                        info!("failed to deserialize Veilid Message");
                    }
                }
            }
            VeilidUpdate::RouteChange(route_change) => {
                info!("Recieved route change message");
                for route_id in route_change.dead_routes {
                    if let Some(incoming_route_id) = self.incoming_route {
                        if incoming_route_id == route_id {
                            info!("incoming route died");
                            self.incoming_route = None;
                            self.install_route().await;
                        }
                    }
                }
                for route_id in route_change.dead_remote_routes {
                    info!("removing {:?} from outgoing routes", route_id.to_string());
                    if let Some((rel, route_id)) = self.outgoing_routes.remove_by_right(&route_id) {
                        self.store_expired_route_id(&rel, route_id).await;
                    }
                }
            }

            _ => {}
        }
    }

    async fn handle_veilid_message(&mut self, rel: Relation, msg: VeilidMessage) {
        debug!("Handling Veilid message");
        match msg {
            // A generated invite code was used by some node to establish a connection.
            // This needs to be verified first
            VeilidMessage::CompleteInvite { dht_key, code } => {
                info!("completing invite");
                // test if the code is valid, if so add dht/rel to lists
                if self.remove_invite_from_settings(code).await {
                    info!("found invite code, storing dht information");
                    self.store_outgoing_dht(&rel, dht_key).await;
                    // Inform directory of new relation
                    let msg = RouterProcessorMessage::SetDirectoryEntry(
                        rel.clone(),
                        "veilid_enabled".into(),
                        "true".into(),
                    );
                    let msg = ProcessorMessage::RouterMessage(msg);
                    self.pl.send(msg).await;
                    // Send name to peer
                    let name = self.pl.state().name().await.clone();
                    let msg = RouterMessage::SetIdentityProperty("name".into(), name);
                    let msg = Message::Router(msg);
                    self.handle_route_message(rel, msg).await;
                } else {
                    info!("No such invite code, rejecting invite completion");
                }
            }
            VeilidMessage::Message(msg) => {
                info!("Recieved message");
                // Detect if this relation is from a node we have a link to.
                if self.load_outgoing_dht(&rel).await.is_some() {
                    info!("the relation is known");
                    self.pl
                        .send(ProcessorMessage::RemoteMessage(rel, msg))
                        .await;
                } else {
                    info!("The relation is not known, msg rejected");
                }
            }
        }
    }

    async fn recv_control_msg(&mut self, control_msg: VeilidProcessorMessage) {
        info!("Recieving veilid control message: {:?}", control_msg);
        match control_msg {
            VeilidProcessorMessage::AcceptInvite(invite) => {
                self.handle_accept_invite(invite).await;
            }
            VeilidProcessorMessage::RouteMessage(rel, msg) => {
                self.handle_route_message(rel, msg).await;
            }
            VeilidProcessorMessage::VeilidEnabled(rel, dht_key) => {
                info!("Veilid recvd VeilidEnabled");
                self.store_outgoing_dht(&rel, dht_key).await;
                // Also reply with our own key
                let dht_descriptor = self.load_incoming_dht().await.unwrap();
                let own_dht_key = dht_descriptor.key().clone();
                let msg = RouterMessage::VeilidEnabled(own_dht_key);
                let msg = Message::Router(msg);
                info!("Veilid Replied to Veilid Enabled");
                self.pl.send_message(rel, msg).await;
                // self.handle_route_message(rel, msg).await;
            }
            VeilidProcessorMessage::GenerateInvite(rel) => {
                info!("Generating invite inner...");
                // Create invite structure
                let dht_key = self
                    .load_incoming_dht()
                    .await
                    .expect("DHT should be established before processing");
                let dht_key = dht_key.key().clone();

                let code = rand::thread_rng().gen();
                let invite = self.add_invite_to_settings(dht_key, code).await;
                // Forward invite to source that requested it
                if let Some(rel) = rel {
                    info!("Returning invite...");
                    let _ = self
                        .pl
                        .send_message(rel, Message::Router(RouterMessage::Invite(invite)))
                        .await;
                }
            }
            VeilidProcessorMessage::RevokeInvite(string_id) => {
                if let Ok(code) = u64::from_str(&string_id) {
                    self.remove_invite_from_settings(code).await;
                }
            }
            VeilidProcessorMessage::Upkeep => {
                self.install_route().await;
            }
        }
    }

    /// Handle the [VeilidProcessorMessage::AcceptInvite] message to incorporate
    /// an invite code and complete the invite process by sending a
    /// [VeilidMessage::CompleteInvite] to the originator of the code to
    /// complete the invite process.
    async fn handle_accept_invite(&mut self, invite: VeilidInvite) {
        if !invite.verify() {
            info!("Invite invalid");
            // Invite is not valid, ignore it for now
            return;
        }

        let routing = self.api.routing_context().unwrap();

        let dht_key = invite.dht_key();
        routing.open_dht_record(*dht_key, None).await.unwrap();
        let dht_result = routing.get_dht_value(*dht_key, 0, true).await;
        if let Err(e) = &dht_result {
            info!("DHT FAILED WITH ERROR: {:?}", e);
        }
        if let Ok(Some(dht_value)) = dht_result {
            info!("Got DHT value");
            if let Ok(route_id) = self
                .api
                .import_remote_private_route(dht_value.data().to_vec())
            {
                info!("Imported remote route blob");
                let us = self.pl.state().self_relation().await;
                let our_dht_key = self.load_incoming_dht().await.unwrap();
                let our_dht_key = our_dht_key.key().clone();
                let code = invite.code();
                // send the complete invite message
                let frames = VeilidFrame::new_complete_invite(&us, our_dht_key, code);
                for frame in frames {
                    let data = serde_json::to_vec(&frame).unwrap();
                    if routing
                        .app_message(Target::PrivateRoute(route_id), data)
                        .await
                        .is_err()
                    {
                        info!("failed to route invite completion");
                        return;
                    }
                }
                info!("Send invite completion, adding data to lists");
                // add the details to our lists
                self.store_outgoing_dht(invite.rel(), *dht_key).await;
                self.outgoing_routes.insert(invite.rel().clone(), route_id);
                // Add to directory
                let msg = RouterProcessorMessage::SetDirectoryEntry(
                    invite.rel().clone(),
                    "veilid_enabled".into(),
                    "true".into(),
                );
                let msg = ProcessorMessage::RouterMessage(msg);
                self.pl.send(msg).await;
                // send our name to peer
                let name = self.pl.state().name().await.clone();
                let msg = RouterMessage::SetIdentityProperty("name".into(), name);
                let msg = Message::Router(msg);
                self.handle_route_message(invite.rel().clone(), msg).await;
            } else {
                info!("failed to import private route blob");
            }
        } else {
            info!("failed to read from DHT");
        }
    }

    /// Creates a new Route and sends it to the DHT record.
    async fn install_route(&mut self) -> Option<()> {
        info!("incoming route: {:?}", self.incoming_route);
        if self.incoming_route.is_some() {
            // dont install route if it already exists
            info!("skipping route install because it already exists");
            return Some(());
        }
        info!("Installing new incoming route");
        let (route_id, new_route_blob) = self.api.new_private_route().await.ok()?;

        let dht_descriptor = self
            .load_incoming_dht()
            .await
            .expect("DHT keys should be generated before operation");
        let writer = Some(KeyPair::new(
            dht_descriptor.owner().clone(),
            *dht_descriptor.owner_secret().unwrap(),
        ));
        let dht_key = dht_descriptor.key().clone();
        info!("our dht_key: {:?}", dht_key);

        // store the new blob
        let routing = self.api.routing_context().expect("Api should be started");
        info!("Opening dht record");
        routing.open_dht_record(dht_key, writer).await.ok()?;
        info!("Setting DHT value to {:?}", new_route_blob);
        let result = routing
            .set_dht_value(dht_key, 0, new_route_blob, None)
            .await;
        info!("Set DHT result: {:?}", result);
        self.incoming_route = Some(route_id);
        Some(())
    }

    // Message handler functions

    async fn handle_route_message(&mut self, rel: Relation, msg: Message) {
        info!("Routing message");
        let routing = self.api.routing_context().unwrap();
        let us = self.pl.state().self_relation().await;

        // Get route_id
        let limit = 8;
        let mut tries = 0;
        let route_id = loop {
            let route_id = match self.outgoing_routes.get_by_left(&rel) {
                Some(route_id) => Some(route_id.clone()),
                None => {
                    info!("route_id missing");
                    // The route was not active, must activate it first
                    if let Some(dht_key) = self.load_outgoing_dht(&rel).await {
                        info!("got DHT key from store: {:?}", dht_key);
                        // Get the route id from the DHT
                        if routing.open_dht_record(dht_key, None).await.is_err() {
                            info!("failed to open DHT from the network");
                        }
                        let dht_result = routing.get_dht_value(dht_key, 0, true).await;
                        if let Ok(Some(dht_value)) = dht_result {
                            info!("Got DHT value: {:?}", dht_value.data().to_vec());
                            if let Ok(route_id) = self
                                .api
                                .import_remote_private_route(dht_value.data().to_vec())
                            {
                                if Some(route_id) == self.load_expired_route_id(&rel).await {
                                    None
                                } else {
                                    info!("Imported private route");
                                    // store the route for next message
                                    info!("message sent, storing new route_id");
                                    self.outgoing_routes.insert(rel.clone(), route_id.clone());
                                    Some(route_id)
                                }
                            } else {
                                // failed to construct route_id from dht data
                                info!("failed to convert blob to route");
                                None
                            }
                        } else {
                            info!("Failed to read DHT value");
                            // failed, drop message for now
                            None
                        }
                    } else {
                        // There is no such relation known to us, ignore
                        info!("No DHT entry in store");
                        None
                    }
                }
            };
            if let Some(route_id) = route_id {
                debug!("found route_id!");
                break route_id;
            }
            if tries >= limit {
                debug!("Number of tries exceeded");
                return;
            }
            tokio::time::sleep(Duration::from_secs(8)).await;
            tries += 1;
        };

        // Send message
        let frames = VeilidFrame::new_wrapped_msg(&us, msg);
        println!("message split into {} frames", frames.len());
        for frame in frames {
            println!("frame data length: {}", frame.data_len());
            let data = serde_json::to_vec(&frame).unwrap();
            println!("frame serialized length: {}", data.len());
            println!(
                "sending frame {} of {} in sequence",
                frame.index(),
                frame.count()
            );
            let res = routing
                .app_message(Target::PrivateRoute(route_id), data)
                .await;
            info!("message result: {:?}", res);
        }

        // if let Some(route_id) = self.outgoing_routes.get_by_left(&rel) {
        //     info!("route_id exists {}", route_id.to_string());
        //     // Send the message with the route
        //     let frames = VeilidFrame::new_wrapped_msg(&us, msg);
        //     println!("message split into {} frames", frames.len());
        //     for frame in frames {
        //         println!("frame data length: {}", frame.data_len());
        //         let data = serde_json::to_vec(&frame).unwrap();
        //         println!("frame serialized length: {}", data.len());
        //         let res = routing
        //             .app_message(Target::PrivateRoute(*route_id), data)
        //             .await;
        //         info!("message result: {:?}", res);
        //     }
        // } else {
        //     info!("route_id missing");
        //     // The route was not active, must activate it first
        //     if let Some(dht_key) = self.load_outgoing_dht(&rel).await {
        //         info!("got DHT key from store: {:?}", dht_key);
        //         // Get the route id from the DHT
        //         if routing.open_dht_record(dht_key, None).await.is_err() {
        //             info!("failed to open DHT from the network");
        //             return;
        //         }
        //         let dht_result = routing.get_dht_value(dht_key, 0, true).await;
        //         if let Ok(Some(dht_value)) = dht_result {
        //             info!("Got DHT value: {:?}", dht_value.data().to_vec());
        //             if let Ok(route_id) = self
        //                 .api
        //                 .import_remote_private_route(dht_value.data().to_vec())
        //             {
        //                 info!("Imported private route");
        //                 // send the message with the route
        //                 let frames = VeilidFrame::new_wrapped_msg(&us, msg);
        //                 for frame in frames {
        //                     let data = serde_json::to_vec(&frame).unwrap();
        //                     let res = routing
        //                         .app_message(Target::PrivateRoute(route_id.into()), data)
        //                         .await;
        //                     debug!("message result: {:?}", res);
        //                 }
        //                 // store the route for next message
        //                 info!("message sent, storing new route_id");
        //                 self.outgoing_routes.insert(rel, route_id);
        //             } else {
        //                 // failed to construct route_id from dht data
        //                 info!("failed to convert blob to route");
        //             }
        //         } else {
        //             info!("Failed to read DHT value");
        //             // failed, drop message for now
        //         }
        //     } else {
        //         // There is no such relation known to us, ignore
        //         info!("No DHT entry in store");
        //     }
        // }
    }

    // Invite management helper functions

    /// Creates a [VeilidInvite], adds it to the internal list of invites,
    /// adds it to the settings page for invites.
    async fn add_invite_to_settings(&mut self, dht_key: TypedKey, code: u64) -> Invite {
        let us = self.pl.state().self_relation().await;
        let invite = VeilidInvite::new(&us, dht_key, code);
        // let invite = Invite::Veilid(invite);
        self.add_pending_invite(code).await;
        // Add invite to pending invite list
        let msg = UiProcessorMessage::SetSetting {
            header: "Pending Connections".into(),
            title: format!("Veilid Invite: {}", code),
            inputs: vec![
                ("button".into(), "View".into()),
                ("button".into(), "Revoke".into()),
            ],
            cb: |e| {
                let invite: VeilidInvite = serde_json::from_str(&e.data()).unwrap();
                // View
                if e.index() == 0 {
                    let invite = Invite::Veilid(invite);
                    let msg = RouterMessage::Invite(invite);
                    let msg = Message::Router(msg);
                    let msg = RouterProcessorMessage::SendMessage(e.rel().clone(), msg);
                    let msg = ProcessorMessage::RouterMessage(msg);
                    return Some(msg);
                }
                // Revoke
                else if e.index() == 1 {
                    let revoke_id = invite.code().to_string();
                    let msg = RouterProcessorMessage::RevokeInvite(revoke_id);
                    let msg = ProcessorMessage::RouterMessage(msg);
                    return Some(msg);
                }
                None
            },
            data: serde_json::to_string(&invite).unwrap(),
        };
        self.pl.send(ProcessorMessage::UiMessage(msg)).await;
        Invite::Veilid(invite)
    }

    async fn remove_invite_from_settings(&mut self, code: u64) -> bool {
        // remove the entry from the veilid list, getting the route blob
        if self.remove_pending_invite(code).await {
            // Remove it from the settings list
            let msg = UiProcessorMessage::RemoveSetting {
                header: "Pending Connections".into(),
                title: format!("Veilid Invite: {}", code),
            };
            self.pl.send_ui(msg).await;
            true
        } else {
            false
        }
    }

    // ============ Local Store Items ===============

    /// Retrieves the keypair for this node
    async fn get_own_keypair(&self) -> Option<CryptoTyped<KeyPair>> {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        let key_string = store.load_user_secret_string("own_keys").await.ok()?;
        key_string.map(|key_string| CryptoTyped::from_str(&key_string).ok())?
    }

    /// Stores the DHT key for this node
    async fn store_incoming_dht(&self, dht_key: DHTRecordDescriptor) {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("incoming_dht", 1).await.unwrap();
        table.store_json(0, b"own_dht", &dht_key).await;
    }

    /// Loads the DHT key for this node
    async fn load_incoming_dht(&self) -> Option<DHTRecordDescriptor> {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("incoming_dht", 1).await.unwrap();
        table.load_json(0, b"own_dht").await.ok()?
    }

    /// Store the DHT info for an outgoing connection
    /// The data is stored as Map<Relation, DHTKey>
    async fn store_outgoing_dht(&self, rel: &Relation, dht_key: TypedKey) {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("outgoing_dht", 1).await.unwrap();
        table
            .store_json(0, rel.to_base64().as_bytes(), &dht_key)
            .await;
    }

    // Store expired route ids
    /// Set the expired route_id for this relation
    async fn store_expired_route_id(&self, rel: &Relation, route_id: RouteId) {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("expired_routes", 1).await.unwrap();
        table
            .store_json(0, rel.sha256().as_bytes(), &route_id)
            .await;
    }

    /// Loads the expired route_id for this relation
    async fn load_expired_route_id(&self, rel: &Relation) -> Option<RouteId> {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("expired_routes", 1).await.unwrap();
        table.load_json(0, rel.sha256().as_bytes()).await.ok()?
    }

    /// Load the DHT info for an outgoing connection
    /// The data is stored as Map<Relation, DHTKey>
    async fn load_outgoing_dht(&self, rel: &Relation) -> Option<TypedKey> {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("outgoing_dht", 1).await.unwrap();
        table.load_json(0, rel.to_base64().as_bytes()).await.ok()?
    }

    // Pending invites section
    // Pending invites are a map from a RouteId to its blob form
    async fn load_pending_invites(&self) -> Vec<VeilidInvite> {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("pending_invites", 1).await.unwrap();
        let invite_codes = table.get_keys(0).await.unwrap();
        info!("got keys: {:?}", invite_codes);
        let us = self.pl.state().self_relation().await;
        let dht_key = self
            .load_incoming_dht()
            .await
            .expect("dht should be generated before operation");
        let dht_key = dht_key.key().clone();
        let mut result = Vec::with_capacity(invite_codes.len());
        for invite_code in invite_codes {
            let invite_code =
                u64::from_be_bytes(invite_code.try_into().expect("code should be 8 bytes long"));
            let invite = VeilidInvite::new(&us, dht_key, invite_code);
            result.push(invite);
        }
        result
    }

    async fn add_pending_invite(&self, code: u64) {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("pending_invites", 1).await.unwrap();
        table.store(0, &code.to_be_bytes(), &[1]).await;
    }

    async fn remove_pending_invite(&self, code: u64) -> bool {
        let store = self
            .api
            .table_store()
            .expect("Veilid api should be started");
        let table = store.open("pending_invites", 1).await.unwrap();
        table
            .delete(0, &code.to_be_bytes())
            .await
            .ok()
            .flatten()
            .is_some()
    }
}

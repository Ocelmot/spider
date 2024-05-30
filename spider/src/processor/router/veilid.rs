use std::{collections::HashSet, sync::Arc};

use spider_link::{
    message::{Invite, InviteType, Message, RouterMessage, VeilidInvite, VeilidMessage},
    Relation,
};
use tokio::{
    select,
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle},
};
use veilid_core::{
    Crypto, CryptoKey, CryptoKind, CryptoTyped, DHTSchema, DHTSchemaSMPL, DHTSchemaSMPLMember,
    FromStr, KeyPair, RouteId, Target, VeilidAPI, VeilidUpdate,
};

use crate::processor::{link::ProcessorLink, message::ProcessorMessage, ui::UiProcessorMessage};

use super::RouterProcessorMessage;

pub enum VeilidProcessorMessage {
    Invite(VeilidInvite),
    RouteMessage(Relation, Message),
    GenerateInvite(Option<Relation>),
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
    veilid_rx: Receiver<VeilidUpdate>,
}

impl VeilidProcessorState {
    async fn new(pl: ProcessorLink, receiver: Receiver<VeilidProcessorMessage>) -> Option<Self> {
        let (veilid_tx, veilid_rx) = channel(50);
        let api = veilid_core::api_startup(
            Arc::new(move |arg| {
                let _ = veilid_tx.blocking_send(arg);
            }),
            Arc::new(|arg| Ok(Box::new(()))),
        )
        .await;
        let api = match api {
            Ok(api) => api,
            Err(e) => {
                eprintln!("VeilidProcessor encountered error: {}", e);
                return None;
            }
        };

        Some(Self {
            pl,
            receiver,
            api,
            veilid_rx,
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
                }
            }
            self.api.detach().await;
        });
        handle
    }

    async fn init(&mut self) {
        let store = self.api.protected_store().unwrap();
        let key = self.get_own_keypair().await;
        if key.is_none() {
            // Generate a new key
            let keypair = Crypto::generate_keypair(CryptoKind::default())
                .expect("should be able to generate keys");
            store.save_user_secret_string("own_keys", keypair.to_string());
        }

        self.api.attach().await;
    }

    async fn recv_veilid_msg(&mut self, veilid_update: VeilidUpdate) {
        if let VeilidUpdate::AppMessage(msg) = veilid_update {
            if let Some(route_id) = msg.route_id() {
                if let Some(rel) = self.load_reverse_lookup(route_id).await {
                    match serde_json::from_slice(msg.message()) {
                        Ok(msg) => {
                            let msg = ProcessorMessage::RemoteMessage(rel, msg);
                            self.pl.send(msg).await;
                        }
                        Err(err) => {
                            eprintln!("Velid encountered error deserializing: {:?}", err);
                        }
                    }
                } else {
                    println!("Reverse lookup lost the relation");
                }
            }
        }
    }

    async fn recv_control_msg(&mut self, control_msg: VeilidProcessorMessage) {
        match control_msg {
            VeilidProcessorMessage::Invite(invite) => {
                let routing = self.api.routing_context().unwrap();
                let invite_route = self
                    .api
                    .import_remote_private_route(invite.route().to_vec())
                    .expect("Invalid route blob");

                // Generate DHT entry
                let own_keys = self
                    .get_own_keypair()
                    .await
                    .expect("own_keys should be initialized");
                let members = vec![
                    DHTSchemaSMPLMember {
                        m_key: own_keys.key,
                        m_cnt: 1,
                    },
                    DHTSchemaSMPLMember {
                        m_key: invite.id().clone(),
                        m_cnt: 1,
                    },
                ];
                let schema = DHTSchema::SMPL(
                    DHTSchemaSMPL::new(0, members).expect("Schema creation should succeed"),
                );
                let dht = routing
                    .create_dht_record(schema, None)
                    .await
                    .expect("DHT entry creation didnt succeed");
                // set route blob into dht
                let (route_id, route_blob) = self.api.new_private_route().await.expect("");
                routing.set_dht_value(*dht.key(), 1, route_blob, None).await;
                // Send DHT entry through route
                let key = dht.key();
                let us = self.pl.state().self_relation().await;
                let message = VeilidMessage::complete_invite(&us, own_keys.key, key.clone());
                let message = serde_json::to_vec(&message).unwrap();
                routing
                    .app_message(Target::PrivateRoute(invite_route), message)
                    .await;
                // Store Map<Relation, (VeilidId, RouteId)>
                self.store_connection_params(invite.rel(), key.clone(), route_id)
                    .await;
            }
            VeilidProcessorMessage::RouteMessage(rel, msg) => {
                if let Some((_, route_id)) = self.load_connection_params(&rel).await {
                    let routing = self.api.routing_context().unwrap();
                    let data = serde_json::to_vec(&msg).unwrap();
                    routing
                        .app_message(Target::PrivateRoute(route_id), data)
                        .await;
                }
            }
            VeilidProcessorMessage::GenerateInvite(rel) => {
                // Create route
                let (route_id, route_blob) = self.api.new_private_route().await.unwrap();

                // Create invite structure
                let id = self
                    .get_own_keypair()
                    .await
                    .expect("Keypair should be stored")
                    .key;
                let us = self.pl.state().self_relation().await;
                let invite = VeilidInvite::new(&us, id, route_blob);
                self.add_pending_invite(route_id).await;
                // Add invite to pending invite list
                let msg = UiProcessorMessage::SetSetting {
                    header: "Pending Connections".into(),
                    title: vec!["Veilid Invite: ".into(), invite.to_base64()].concat(),
                    inputs: vec![],
                    cb: |_idx, _name, _input, _data| None,
                    data: String::new(),
                };
                self.pl.send(ProcessorMessage::UiMessage(msg)).await;
                // Forward invite to source that requested it
                if let Some(rel) = rel {
                    let _ = self
                        .pl
                        .send_message(
                            rel,
                            Message::Router(RouterMessage::Invite(Invite::Veilid(invite))),
                        )
                        .await;
                }
            }
        }
    }

    // ============ Local Store Items ===============

    async fn get_own_keypair(&self) -> Option<KeyPair> {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        let key_string = store.load_user_secret_string("own_keys").await.ok()?;
        key_string.map(|key_string| KeyPair::from_str(&key_string).ok())?
    }

    async fn store_connection_params(
        &self,
        rel: &Relation,
        key: CryptoTyped<CryptoKey>,
        route: RouteId,
    ) {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        // remove old reverse lookup
        if let Some((_, route_id)) = self.load_connection_params(rel).await {
            store.remove_user_secret(route_id.to_string()).await;
        }

        // Save the user secret
        store
            .save_user_secret_json(rel.to_base64(), &(key, route))
            .await
            .unwrap();
        store
            .save_user_secret_json(route.to_string(), &rel)
            .await
            .unwrap();
    }
    async fn load_connection_params(
        &self,
        rel: &Relation,
    ) -> Option<(CryptoTyped<CryptoKey>, RouteId)> {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        store
            .load_user_secret_json(rel.to_base64())
            .await
            .ok()
            .flatten()
    }
    async fn load_reverse_lookup(&self, route_id: &RouteId) -> Option<Relation> {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        store
            .load_user_secret_json(route_id.to_string())
            .await
            .ok()
            .flatten()
    }
    // Pending invites section
    async fn load_pending_invites(&self) -> HashSet<RouteId> {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        store
            .load_user_secret_json("pending_invites")
            .await
            .unwrap_or_default()
            .unwrap_or_default()
    }
    async fn add_pending_invite(&self, route_id: RouteId) {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        let mut pending_invites = self.load_pending_invites().await;
        pending_invites.insert(route_id);
        store
            .save_user_secret_json("pending invites", &pending_invites)
            .await
            .unwrap();
    }
    async fn test_pending_invite(&self, route_id: &RouteId) -> bool {
        let pending_invites = self.load_pending_invites().await;
        pending_invites.contains(route_id)
    }
    async fn remove_pending_invite(&self, route_id: &RouteId) {
        let store = self
            .api
            .protected_store()
            .expect("Veilid api should be started");
        let mut pending_invites = self.load_pending_invites().await;
        pending_invites.remove(route_id);
        store
            .save_user_secret_json("pending invites", &pending_invites)
            .await
            .unwrap();
    }
}

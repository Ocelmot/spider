use core::panic;
use std::{path::PathBuf, time::Duration};

use spider_link::{
    beacon::Beacon,
    link::{LinkSet, LinkSetMsg, TCPLink},
    message::{Message, RouterMessage},
};
use tokio::{
    select, spawn,
    sync::mpsc::{channel, unbounded_channel, Receiver, UnboundedSender},
    task::JoinHandle,
};
use tracing::{error, info, trace};
// use veilid_core::{VeilidConfigInner, VeilidConfigNetwork};

use crate::{
    client_message::{ClientControl, ClientResponse},
    error::{ClientError, ClientResult, ErrorKind, Problem, ProblemWrap},
    state::SpiderClientState,
    // veilid_hub_registry::RegistryEntry,
    ClientChannel, SpiderClientBuilder,
};

pub(crate) struct SpiderClientProcessor {
    state_path: Option<PathBuf>,
    state: SpiderClientState,
    link_set: Option<LinkSet>,
    beacon: Beacon,
    client_channel: ClientChannel,
    receiver: Receiver<ClientControl>,
    on_message: Option<Box<dyn FnMut(&ClientChannel, Message, u64) + Send>>,
    on_connect: Option<Box<dyn FnMut(&ClientChannel, u64) + Send>>,
    on_disconnect: Option<Box<dyn FnMut(&ClientChannel) + Send>>,
    on_terminate: Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>,
    channels: Vec<UnboundedSender<ClientResponse>>,
}

impl SpiderClientProcessor {
    pub(crate) async fn start_processor(
        state_path: Option<PathBuf>,
        state: SpiderClientState,
        enable_recv: bool,
    ) -> ClientResult<(ClientChannel, JoinHandle<ClientResult>)> {
        trace!("Starting Client Processor");

        let id = state.self_relation.relation.id.clone();

        let (sender, receiver) = channel(50);
        let (client_channel, channels) = if enable_recv {
            let (tx, rx) = unbounded_channel();
            let client_channel = ClientChannel::with_receiver(id, sender, rx);
            (client_channel, vec![tx])
        } else {
            let client_channel = ClientChannel::new(id, sender);
            (client_channel, Vec::new())
        };

        let mut processor = Self {
            state_path,
            state,
            link_set: None,
            beacon: Beacon::new(Duration::from_secs(10)),
            client_channel: client_channel.clone(),
            receiver,
            on_message: None,
            on_connect: None,
            on_disconnect: None,
            on_terminate: None,
            channels,
        };

        let handle = spawn(async move {
            processor
                .init_link_set()
                .await
                .problem_msg(ErrorKind::Misc, "Failed to initialize client processor")?;
            processor.run().await
        });
        Ok((client_channel, handle))
    }

    async fn init_link_set(&mut self) -> ClientResult {
        let Some(host_relation) = &self.state.host_relation else {
            return Ok(());
        };

        let link_set = LinkSet::new(self.state.self_relation.clone(), host_relation.clone());

        // enable reconnect capability
        if self.state.auto_reconnect {
            link_set
                .set_reconnect(true)
                .await
                .wrap_msg("failed to set reconnect")?;
            link_set
                .set_allow_reconnect(Some(5))
                .await
                .wrap_msg("failed to set allow_reconnect")?;
        }

        // enable TCP link capability
        link_set
            .add_connector(TCPLink::connect)
            .await
            .wrap_msg("failed to add_connector")?;

        // Enable fixed addr capability
        if self.state.fixed_addr_enable {
            for addr in &self.state.fixed_addrs {
                link_set
                    .add_addr(addr.clone())
                    .await
                    .wrap_msg("failed to add fixed addr")?;
            }
        }

        // Enable last_addr capability
        if self.state.base_addrs_enable {
            for (addr, _) in &self.state.base_addrs {
                link_set
                    .add_addr(addr.clone())
                    .await
                    .wrap_msg("failed to add base addr")?;
            }
        }

        // enable veilid capability
        // if let Some(veilid_name) = &self.state.veilid_enable {
        //     let config = get_veilid_config(&self.state, veilid_name.clone());

        //     let mut reg_entry = RegistryEntry::get_or_create_entry(
        //         self.state.self_relation.clone(),
        //         self.state.veilid_own_dht.clone(),
        //         config,
        //     )
        //     .await
        //     .wrap_msg("Failed to get VeilidHub from registry")?;

        //     if reg_entry.has_listener_callback().await {
        //         return Err(ClientError::new().msg("A client with this SelfRelation is already connected to the Veilid system. Only one such client is allowed at a time."));
        //     }

        //     if self.state.veilid_own_dht.is_none() {
        //         self.state.veilid_own_dht = Some(reg_entry.get_listen_dht().clone());
        //         if let Some(state_path) = &self.state_path {
        //             self.state
        //                 .to_file(state_path)
        //                 .await
        //                 .wrap_msg("failed to save veilid dht")?;
        //         }
        //     }

        //     let connector = reg_entry.get_connector();
        //     link_set
        //         .add_connector(move |_sr, r, addr| {
        //             let fut_connector = connector.clone();
        //             async move { fut_connector.connect(r, addr).await }
        //         })
        //         .await
        //         .wrap_msg("failed to add veilid connector")?;

        //     let listen_sender = link_set.clone_sender();
        //     reg_entry
        //         .set_listener_callback(move |link| {
        //             let async_sender = listen_sender.clone_sender();
        //             async move {
        //                 let _ = async_sender.add_link(link).await;
        //             }
        //         })
        //         .await;
        // }

        self.link_set = Some(link_set);
        Ok(())
    }

    async fn dispose_link_set(&mut self, _link_set: LinkSet) -> ClientResult {
        // if let Some(veilid_name) = &self.state.veilid_enable {
        //     let config = get_veilid_config(&self.state, veilid_name.clone());

        //     let mut reg_entry = RegistryEntry::get_or_create_entry(
        //         self.state.self_relation.clone(),
        //         self.state.veilid_own_dht.clone(),
        //         config,
        //     )
        //     .await
        //     .wrap_msg("Failed to get VeilidHub from registry")?;

        //     reg_entry.clear_listener_callback().await;
        // }
        Ok(())
    }

    async fn run(mut self) -> ClientResult {
        // operate the connection, sending and returning
        let mut connected = false;
        let mut reconnecting = false;

        loop {
            trace!(
                "connected: {connected}, link_set.is_some() {}, beacon_enable: {}",
                self.link_set.is_some(),
                self.state.beacon_enable
            );
            select! {
                res = opt_link_set_recv(self.link_set.as_mut()) => {
                    trace!("Client received message from link set. is some: {}", res.is_ok());
                    let Ok((link_set, link_msg)) = res else {
                        info!("Link died, restarting");
                        // Link has died, restart it
                        let _ = self.init_link_set().await;
                        continue;
                    };
                    match link_msg {
                        LinkSetMsg::Disconnected => {
                            connected = false;
                            self.process_client_response(ClientResponse::Disconnected).await;
                        },
                        LinkSetMsg::Connected(epoch) => {
                            connected = true;
                            self.process_client_response(ClientResponse::Connected(epoch)).await;
                        },
                        LinkSetMsg::Connecting(re_con) => {
                            reconnecting = re_con;
                        }
                        LinkSetMsg::Message(message, epoch) => {
                            // if the connection is pending, send saved permission code if available
                            if let Message::Router(RouterMessage::Pending) = &message {
                                if let Some(code) = &self.state.permission_code {
                                    let msg = RouterMessage::ApprovalCode(code.clone());
                                    let msg = Message::Router(msg);
                                    let _ = link_set.send_with_epoch(msg, epoch).await;
                                }
                            }

                            if let Message::Router(RouterMessage::Addrs(addrs)) = &message {
                                if self.state.base_addrs_enable {
                                    for addr in addrs{
                                        trace!("Adding addr to set of base addrs {}", addr);
                                        self.state.base_addrs.push(addr.clone(), ());
                                        let _ = link_set.add_addr(addr.clone()).await;
                                    }
                                    let _ = self.save_state().await;
                                }
                            }

                            // if denied close the connection with the denied response
                            if let Message::Router(RouterMessage::Denied) = &message {
                                info!("Connection denied");
                                let old_relation = self.state.host_relation.take().expect("connected clients should have a host relation");
                                if let Some(link_set) = self.link_set.take() {
                                    self.dispose_link_set(link_set).await?;
                                }
                                self.beacon.clear_sockets();
                                self.process_client_response(ClientResponse::Disconnected).await;
                                self.process_client_response(ClientResponse::Unpaired(old_relation)).await;
                                continue;
                            }

                            self.process_client_response(ClientResponse::Message(message, epoch)).await;
                        },
                    }
                },

                ctrl_msg = self.receiver.recv() => {
                    trace!("Client received control message. is_some: {}", ctrl_msg.is_some());
                    match ctrl_msg {
                        Some(ctrl_msg) => {
                            match ctrl_msg {
                                ClientControl::Pair(rel) => {
                                    trace!("Pairing");
                                    if self.state.host_relation.is_none() {
                                        self.state.host_relation = Some(rel);
                                        self.init_link_set().await.wrap_msg("Failed to initialize link set")?;
                                        let _ = self.save_state().await;
                                        self.process_client_response(ClientResponse::Paired).await;
                                    }else{
                                        trace!("Already had host relation");
                                    }
                                },
                                ClientControl::Connect => {
                                    // trying to connect an unpaired client does
                                    // nothing
                                    if let Some(link_set) = &self.link_set {
                                        let _ = link_set.connect().await;
                                    }
                                }
                                ClientControl::Message(msg, epoch) => {
                                    // Messages sent while unpaired are not meaningful
                                    if let Some(link_set) = &self.link_set {
                                        let _ = link_set.send_opt_epoch(msg, epoch).await;
                                    }
                                },
                                ClientControl::Disconnect => {
                                    // trying to disconnect an unpaired client
                                    // does nothing
                                    if let Some(link_set) = &self.link_set {
                                        let _ = link_set.disconnect().await;
                                    }
                                }
                                ClientControl::Unpair => {
                                    // can only unpair if we are already paired
                                    if let Some(old_relation) = self.state.host_relation.take() {
                                        // when unpairing, disconnect if connected
                                        if let Some(link_set) = self.link_set.take() {
                                            self.process_client_response(ClientResponse::Disconnected).await;
                                            self.dispose_link_set(link_set).await?;
                                            connected = false;
                                            self.beacon.clear_sockets();
                                        }
                                        let _ = self.save_state().await;
                                        self.process_client_response(ClientResponse::Unpaired(old_relation)).await;
                                    }
                                },
                                ClientControl::Terminate => {
                                    let builder = SpiderClientBuilder::new(self.state_path.clone(), self.state.clone());
                                    if let Some(link_set) = self.link_set.take() {
                                        self.dispose_link_set(link_set).await?;
                                    }
                                    self.process_client_response(ClientResponse::Terminated(builder)).await;
                                    return Ok(());
                                }

                                ClientControl::AddChannel(ch) => {self.channels.push(ch);},
                                ClientControl::SetOnMessage(cb) => {self.on_message = cb;},
                                ClientControl::SetOnConnect(cb) => {self.on_connect = cb;},
                                ClientControl::SetOnTerminate(cb) => {self.on_terminate = cb;},
                            }
                        },
                        None => {
                            // client channel has terminated, close the client itself as well
                            error!("Client channel closed");
                            let builder = SpiderClientBuilder::new(self.state_path.clone(), self.state.clone());
                            if let Some(link_set) = self.link_set.take() {
                                self.dispose_link_set(link_set).await?;
                            }
                            self.process_client_response(ClientResponse::Terminated(builder)).await;
                            return Ok(());
                        },
                    }
                },
                addr = self.beacon.next_addr(), if (!connected || reconnecting) && self.link_set.is_some() && self.state.beacon_enable => {
                    info!("Client received addr: {:?}", addr);
                    // can only add addrs when the link is paired
                    if let Some(link_set) = &self.link_set{
                        let _ = link_set.try_addr(addr.to_string()).await;
                    }
                }
            }
        }
    }

    async fn process_client_response(&mut self, msg: ClientResponse) {
        // info!("PROCESSING CLIENT RESPONSE! {:?}", msg);
        // find callback
        match &msg {
            ClientResponse::Paired => {
                // TODO: make a callback for this
            }
            ClientResponse::Connected(epoch) => {
                if let Some(cb) = &mut self.on_connect {
                    cb(&self.client_channel, *epoch);
                }
            }
            ClientResponse::Message(msg, epoch) => {
                if let Some(cb) = &mut self.on_message {
                    cb(&self.client_channel, msg.clone(), *epoch);
                }
            }
            ClientResponse::Disconnected => {
                if let Some(cb) = &mut self.on_disconnect {
                    cb(&self.client_channel);
                }
            }
            ClientResponse::Unpaired(_rel) => {
                // TODO: make a callback for this
            }
            ClientResponse::Terminated(builder) => {
                if let Some(cb) = &mut self.on_terminate {
                    cb(builder.clone());
                }
            }
        }

        // send through channels
        self.channels.retain(|ch| match ch.send(msg.clone()) {
            Ok(_) => true,
            Err(_) => false,
        });
    }

    async fn save_state(&mut self) -> ClientResult {
        if let Some(path) = &self.state_path {
            self.state.to_file(path).await?;
        }
        Ok(())
    }
}

async fn opt_link_set_recv(
    ls: Option<&mut LinkSet>,
) -> Result<(&mut LinkSet, LinkSetMsg), ClientError> {
    match ls {
        Some(ls) => {
            let msg = ls.recv().await.wrap()?;
            Ok((ls, msg))
        }
        None => std::future::pending().await,
    }
}

// fn get_veilid_config(state: &SpiderClientState, program_name: String) -> VeilidConfigInner {
//     let config_root = state
//         .veilid_root
//         .as_ref()
//         .map(|path| PathBuf::from(path))
//         .unwrap_or_else(|| PathBuf::from("./.veilid"));

//     VeilidConfigInner {
//         program_name,
//         protected_store: veilid_core::VeilidConfigProtectedStore {
//             directory: {
//                 let path = config_root.join("protected_store");
//                 path.to_str().unwrap().to_string()
//             },
//             ..Default::default()
//         },
//         block_store: veilid_core::VeilidConfigBlockStore {
//             directory: {
//                 let path = config_root.join("block_store");
//                 path.to_str().unwrap().to_string()
//             },
//             ..Default::default()
//         },
//         table_store: veilid_core::VeilidConfigTableStore {
//             directory: {
//                 let path = config_root.join("block_store");
//                 path.to_str().unwrap().to_string()
//             },
//             ..Default::default()
//         },
//         ..Default::default()
//     }
// }

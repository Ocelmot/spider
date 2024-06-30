use core::panic;
use std::{fmt::Debug, path::PathBuf, time::Duration};

use dht_chord::{
    adaptor::{AssociateClient, ChordAdaptor},
    TCPAdaptor,
};
use log::{debug, info};
use spider_link::{
    beacon::beacon_lookout_one,
    message::{Message, RouterMessage},
    Link, SpiderId2048,
};
use tokio::{
    select, spawn,
    sync::mpsc::{channel, unbounded_channel, Receiver, UnboundedSender},
    task::JoinHandle,
    time::{sleep, timeout},
};

use crate::{state::SpiderClientState, SpiderClientBuilder};

use super::{channel::ClientChannel, veilid_link::VeilidLink, ClientControl, ClientResponse};

pub struct SpiderClientProcessor {
    state_path: Option<PathBuf>,
    state: SpiderClientState,
    client_channel: ClientChannel,
    receiver: Receiver<ClientControl>,
    on_message: Option<Box<dyn FnMut(&ClientChannel, Message) + Send>>,
    on_connect: Option<Box<dyn FnMut(&ClientChannel) + Send>>,
    on_disconnect: Option<Box<dyn FnMut(&ClientChannel) + Send>>,
    on_terminate: Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>,
    on_deny: Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>,
    channels: Vec<UnboundedSender<ClientResponse>>,
}

impl SpiderClientProcessor {
    pub(crate) fn start_processor(
        state_path: Option<PathBuf>,
        state: SpiderClientState,
        enable_recv: bool,
    ) -> (ClientChannel, JoinHandle<()>) {
        debug!("Starting Client Processor");
        if state.host_relation.is_none() {
            panic!("Processor requires a host to be able to connect");
        }
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

        let processor = Self {
            state_path,
            state,
            client_channel: client_channel.clone(),
            receiver,
            on_message: None,
            on_connect: None,
            on_disconnect: None,
            on_terminate: None,
            on_deny: None,
            channels,
        };

        let handle = spawn(async move {
            processor.run().await;
        });
        (client_channel, handle)
    }

    /// if the processor is disconnected, attempt to connect
    /// if the processor is connected, process messages
    async fn run(mut self) {
        let mut mode = SpiderClientProcessorMode::Connecting;
        loop {
            debug!("Client Processor is looping with mode={:?}", mode);
            match &mut mode {
                SpiderClientProcessorMode::Connected(conn) => {
                    mode = self.process_conn(conn).await;
                }
                SpiderClientProcessorMode::Connecting => {
                    // reconnect
                    mode = self.connect().await;

                    if let SpiderClientProcessorMode::Connecting = mode {
                        // If they all fail, sleep for 10s
                        self.process_client_response(ClientResponse::Status("Connecting...".into())).await;
                        sleep(Duration::from_secs(10)).await;
                    }

                    if let SpiderClientProcessorMode::Connected(ref mut conn) = mode {
                        if let ConnectionType::Link(link) = conn {
                            let addr = link.peer_addr().clone();
                            if self.state.last_addr_enable {
                                self.state.set_last_addr(Some(addr));
                                self.save_state();
                            }
                        }

                        // establish reconnection subscriptions
                        if self.state.chord_enable {
                            match conn
                                .send(Message::Router(RouterMessage::SubscribeChord(50)))
                                .await
                            {
                                Some(_) => {}
                                None => {
                                    mode = SpiderClientProcessorMode::Connecting;
                                    continue;
                                } // couldnt send message, need to reconnect
                            }
                        }

                        info!("connected =====");
                        if self.state.veilid_enable && conn.is_link() {
                            info!("veilid enabled + conn is link =====");
                            if let Some(own_dht_key) = self.state.own_dht {
                                info!("own_dht set =====");
                                match conn
                                    .send(Message::Router(RouterMessage::VeilidEnabled(own_dht_key)))
                                    .await
                                {
                                    Some(_) => {
                                        info!("Sent VeilidEnabled =====");
                                    }
                                    None => {
                                        mode = SpiderClientProcessorMode::Connecting;
                                        continue;
                                    } // couldnt send message, need to reconnect
                                }
                            }else{
                                info!("own_dht not set =====");
                            }
                        }

                        self.process_client_response(ClientResponse::Connected)
                        .await;
                    }
                }
                SpiderClientProcessorMode::Terminating => {
                    // On terminate
                    let builder = SpiderClientBuilder {
                        state_path: self.state_path.clone(),
                        state: self.state.clone(),
                    };
                    let msg = ClientResponse::Terminated(builder);
                    self.process_client_response(msg).await;
                    break;
                }
            }
        }
    }

    async fn process_conn(&mut self, conn: &mut ConnectionType) -> SpiderClientProcessorMode {
        loop {
            debug!("Processing SpiderClientProcessor connection...");
            select! {
                msg = self.receiver.recv() => {
                    // new message from users
                    match msg {
                        Some(msg) => {
                            if let Some(mode) = self.process_client_control(conn, msg).await {
                                return mode;
                            }
                        },
                        // since the processor itself holds a
                        // sender to the channel, this should not occur
                        None => return SpiderClientProcessorMode::Terminating,
                    }
                },
                msg = conn.recv() => {
                    // new message from base
                    match msg {
                        Some(msg) => {
                            if let Message::Router(RouterMessage::Pending) = &msg {
                                // if we are pending, send saved permission code
                                if let Some(code) = &self.state.permission_code {
                                    let msg = RouterMessage::ApprovalCode(code.clone());
                                    let msg = Message::Router(msg);
                                    conn.send(msg).await;
                                }
                            }
                            if let Message::Router(RouterMessage::Denied) = &msg {
                                let mut builder = SpiderClientBuilder {
                                    state_path: self.state_path.clone(),
                                    state: self.state.clone(),
                                };
                                // Since the connection is denied, remove the host relation
                                builder.state.host_relation = None;
                                self.process_client_response(ClientResponse::Denied(builder)).await;
                                return SpiderClientProcessorMode::Connecting;
                            }
                            if let Message::Router(RouterMessage::ChordAddrs(addrs)) = &msg {
                                info!("RECVD CHORD ADDRS: {:?}", addrs);
                                self.state.chord_addrs = addrs.clone();
                                self.save_state();
                            }
                            if let Message::Router(RouterMessage::VeilidEnabled(dht_key)) = &msg {
                                info!("got new VeilidEnabled message");
                                // The partner has enabled Veilid. If we have veilid enabled,
                                // we should store thier key for future connections.
                                if self.state.veilid_enable {
                                    info!("Saving new paired dht key: {:?}", dht_key);
                                    self.state.paired_dht = Some(*dht_key);
                                    self.save_state();
                                }
                            }
                            self.process_client_response(ClientResponse::Message(msg)).await
                        },
                        None => {
                            // became disconected
                            self.process_client_response(ClientResponse::Disconnected).await;
                            return SpiderClientProcessorMode::Connecting;
                        },
                    }
                }
            }
        }
    }

    fn save_state(&self) {
        if let Some(path) = &self.state_path {
            self.state.to_file(path)
        }
    }

    async fn process_client_control(
        &mut self,
        conn: &mut ConnectionType,
        msg: ClientControl,
    ) -> Option<SpiderClientProcessorMode> {
        match msg {
            ClientControl::Message(msg) => {
                if conn.send(msg).await.is_none() {
                    return Some(SpiderClientProcessorMode::Connecting);
                }
            }
            ClientControl::AddChannel(ch) => {
                self.channels.push(ch);
            }
            ClientControl::SetOnMessage(cb) => {
                self.on_message = cb;
            }
            ClientControl::SetOnConnect(cb) => {
                self.on_connect = cb;
            }
            ClientControl::SetOnTerminate(cb) => {
                self.on_terminate = cb;
            }
            ClientControl::SetOnDeny(cb) => {
                self.on_deny = cb;
            }
            ClientControl::Terminate => {
                return Some(SpiderClientProcessorMode::Terminating);
            }
        }
        None
    }

    async fn process_client_response(&mut self, msg: ClientResponse) {
        // info!("PROCESSING CLIENT RESPONSE! {:?}", msg);
        // find callback
        match &msg {
            ClientResponse::Message(msg) => {
                if let Some(cb) = &mut self.on_message {
                    cb(&self.client_channel, msg.clone());
                }
            }
            ClientResponse::Status(_) => {
                // no callback for this at the moment
            }
            ClientResponse::Connected => {
                if let Some(cb) = &mut self.on_connect {
                    cb(&self.client_channel);
                }
            }
            ClientResponse::Disconnected => {
                if let Some(cb) = &mut self.on_disconnect {
                    cb(&self.client_channel);
                }
            }
            ClientResponse::Terminated(builder) => {
                if let Some(cb) = &mut self.on_terminate {
                    cb(builder.clone());
                }
            }
            ClientResponse::Denied(builder) => {
                if let Some(cb) = &mut self.on_deny {
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

    async fn connect(&mut self) -> SpiderClientProcessorMode {
        debug!("Connecting...");
        debug!("Getting DHT Key");
        self.process_client_response(ClientResponse::Status("Getting DHT key...".into())).await;
        // If Veilid is enabled, get the DHT entry and save it in the client state.
        // This is done first so that a link based connection will be able to 
        // indicate that Veilid is enabled on this device.
        if self.state.veilid_enable {
            if let Some(dht_descriptor) = VeilidLink::get_incoming_dht().await {
                let own_dht = dht_descriptor.key().clone();
                debug!("Loaded incoming DHT entry: {:?}", own_dht);
                self.state.own_dht = Some(own_dht);
                self.save_state();
            }else{
                debug!("Failed to load incoming DHT entry");
            }
        }

        // Try each connection method in turn

        // Last known address
        if self.state.last_addr_enable {
            debug!("Trying last known address");
            self.process_client_response(ClientResponse::Status("Last address...".into())).await;
            let new_mode = timeout(Duration::from_secs(5), async{
                if let Some(addr) = &self.state.last_addr_local {
                    let self_relation = self.state.self_relation.clone();
                    let host_relation = self
                        .state
                        .host_relation
                        .clone()
                        .expect("Host relation should always be set if connected");
                    if let Some(link) = Link::connect(self_relation, addr, host_relation).await {
                        let conn = ConnectionType::Link(link);
                        debug!("Connected via last known local address");
                        return Some(SpiderClientProcessorMode::Connected(conn));
                    }
                }
                if let Some(addr) = &self.state.last_addr_global {
                    let self_relation = self.state.self_relation.clone();
                    let host_relation = self
                        .state
                        .host_relation
                        .clone()
                        .expect("Host relation should always be set if connected");
                    if let Some(link) = Link::connect(self_relation, addr, host_relation).await {
                        let conn = ConnectionType::Link(link);
                        debug!("Connected via last known global address");
                        return Some(SpiderClientProcessorMode::Connected(conn));
                    }
                }
                None
            }).await.ok().flatten();
            if let Some(mode) = new_mode {
                return mode;
            }
        }

        // Beacon
        if self.state.beacon_enable {
            debug!("Trying the beacon...");
            self.process_client_response(ClientResponse::Status("Checking Beacon...".into())).await;
            if let Some(addr) = beacon_lookout_one(Duration::from_secs(5)).await {
                debug!("found beacon addr {:?}", addr);
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");
                if let Some(link) = Link::connect(self_relation, addr.clone(), host_relation).await
                {
                    debug!("Connected via beacon address");
                    let conn = ConnectionType::Link(link);
                    return SpiderClientProcessorMode::Connected(conn);
                } else {
                    debug!("failed to connect using beacon");
                }
            }
        }

        // Veilid
        if self.state.veilid_enable {
            debug!("Trying to connect via Veilid");
            self.process_client_response(ClientResponse::Status("Using Veilid...".into())).await;
            let us = self.state.self_relation.clone();
            let host_relation = self
                .state
                .host_relation
                .clone()
                .expect("Host relation should always be set if connected");
            if let Some(paired_dht) = self.state.paired_dht {
                debug!("Creating Veilid Processor");
                let veilid_processor = VeilidLink::new(us, host_relation, paired_dht).await;
                if let Some(veilid_link) = veilid_processor {
                    debug!("Connected via Veilid link");
                    let conn = ConnectionType::Veilid(veilid_link);
                    return SpiderClientProcessorMode::Connected(conn);
                }else{
                    debug!("Veilid Link not established");
                }
            }else{
                debug!("No paired DHT key");
            }
        } else {
            debug!("Veilid not enabled");
        }

        // Chord
        if self.state.chord_enable {
            debug!("Trying to connect via chord");
            self.process_client_response(ClientResponse::Status("Connecting via chord...".into())).await;
            for addr in &self.state.chord_addrs {
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");

                let mut assoc: AssociateClient<String, SpiderId2048> =
                    TCPAdaptor::associate_client(addr.to_string());
                assoc
                    .send_op(dht_chord::associate::AssociateRequest::GetAdvertOf {
                        id: host_relation.id.clone(),
                    })
                    .await;
                let addr = match timeout(Duration::from_secs(10), assoc.recv_op()).await {
                    Ok(Some(dht_chord::associate::AssociateResponse::AdvertOf {
                        data, ..
                    })) => match data {
                        Some(data) => match String::from_utf8(data) {
                            Ok(addr) => addr,
                            Err(_) => continue,
                        },
                        None => continue,
                    },
                    _ => {
                        continue;
                    }
                };

                if let Some(link) = Link::connect(self_relation, addr.clone(), host_relation).await
                {
                    let conn = ConnectionType::Link(link);
                    return SpiderClientProcessorMode::Connected(conn);
                }
            }
        }

        // Fixed address
        if self.state.fixed_addr_enable {
            debug!("Trying to connect via fixed address");
            self.process_client_response(ClientResponse::Status("Using saved address...".into())).await;
            for addr in &self.state.fixed_addrs {
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");
                if let Some(link) = Link::connect(self_relation, addr, host_relation).await {
                    let conn = ConnectionType::Link(link);
                    return SpiderClientProcessorMode::Connected(conn);
                }
            }
        }

        SpiderClientProcessorMode::Connecting
    }
}

enum SpiderClientProcessorMode {
    /// The processor is disconnected, needs to connect
    Connecting,
    /// The processor is connected with the enclosed [ConnectionType]
    Connected(ConnectionType),
    /// The processor is now exiting
    Terminating,
}

impl Debug for SpiderClientProcessorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connecting => write!(f, "Connecting"),
            Self::Connected(arg0) => {
                let field_string = match arg0 {
                    ConnectionType::Link(_) => "TCP",
                    ConnectionType::Veilid(_) => "Veilid",
                };
                f.debug_tuple("Connected").field(&field_string).finish()
            },
            Self::Terminating => write!(f, "Terminating"),
        }
    }
}

enum ConnectionType {
    /// The processor has found an ip addr to connect to
    Link(Link),
    /// The processor has found a Veilid DHT location to route messages
    Veilid(VeilidLink),
}

impl ConnectionType {
    fn is_link(&self) -> bool {
        if let ConnectionType::Link(_) = self {
            true
        } else {
            false
        }
    }

    fn is_veilid(&self) -> bool {
        if let ConnectionType::Veilid(_) = self {
            true
        } else {
            false
        }
    }

    async fn send(&self, msg: Message) -> Option<()> {
        match self {
            ConnectionType::Link(link) => link.send(msg).await.ok(),
            ConnectionType::Veilid(vlink) => vlink.send(msg).await,
        }
    }

    async fn recv(&mut self) -> Option<Message> {
        match self {
            ConnectionType::Link(link) => link.recv().await,
            ConnectionType::Veilid(vlink) => vlink.recv().await,
        }
    }
}

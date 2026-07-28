use core::panic;
use std::{collections::{hash_map::Entry, HashMap}, path::PathBuf, time::Duration};

#[cfg(feature = "transport_iroh")]
use spider_link::transports::iroh::{IrohHub, SecretKey};
use spider_link::{
    Relation, discovery::{AdvertEvent, Discoverer, get_discovery}, link_set::{Epoch, LinkSet, LinkSetMessage, links::Address}, message::{Message, RouterMessage},
};
use tokio::{
    select, spawn, sync::mpsc::{Receiver, UnboundedSender, channel, unbounded_channel}, task::JoinHandle, time::interval,
};
use tracing::{error, info, trace};

use crate::{
    client_message::{ClientControl, ClientResponse},
    error::{ClientError, ClientResult, ErrorKind, Problem, ProblemWrap},
    state::SpiderClientState,
    ClientChannel, SpiderClientBuilder,
};

pub(crate) struct SpiderClientProcessor {
    state_path: Option<PathBuf>,
    state: SpiderClientState,
    link_set: Option<LinkSet<Message>>,
    discoverer: Option<Box<dyn Discoverer>>,
    #[cfg(feature = "transport_iroh")]
    iroh_hub: Option<IrohHub>,
    client_channel: ClientChannel,
    receiver: Receiver<ClientControl>,
    on_message: Option<Box<dyn FnMut(&ClientChannel, Message, Epoch) + Send>>,
    on_connect: Option<Box<dyn FnMut(&ClientChannel, Epoch) + Send>>,
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

        #[cfg(feature = "transport_iroh")]
        let iroh_hub ={
            use spider_link::transports::iroh::IROH_SCHEME;
            let mut iroh_hub = None;
            if state.transports.contains(IROH_SCHEME) {
                match IrohHub::new(SecretKey::generate()).await {
                    Ok(hub) => iroh_hub = Some(hub),
                    Err(e) => {
                        error!("Failed to initialize Iroh transport: {}", e);
                    }
                }
            }
            iroh_hub
        };

        let mut processor = Self {
            state_path,
            state,
            link_set: None,
            discoverer: None,
            #[cfg(feature = "transport_iroh")]
            iroh_hub,
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

        if self.state.discovery_enable{
            self.discoverer = Some(get_discovery(self.state.beacon_port));
        }

        let link_set = LinkSet::new();

        // The client will try to connect indefinitely
        link_set
            .set_connection_timeout(None)
            .await
            .wrap_msg("Failed to set connection timeout")?;

        // enable reconnect capability
        if self.state.auto_reconnect {
            link_set
                .set_auto_connect(true)
                .await
                .wrap_msg("failed to set reconnect")?;
            // reconnect timeout is low because the base does not attempt a
            // connection to a peripheral
            link_set
                .set_reconnection_timeout(Some(Duration::from_millis(1200)))
                .await
                .wrap_msg("failed to set allow_reconnect")?;
        }

        // enable TCP link capability
        if self.state.transports.contains("auth_tcp") {
            let sr = self.state.self_relation.clone();
            let rel = host_relation.clone();
            let connector = spider_link::transports::tcp::TcpConnector::new(sr, rel);

            link_set
                .add_connector(connector)
                .await
                .wrap_msg("failed to add TCP connector")?;
        }

        #[cfg(feature = "transport_iroh")]
        if self
            .state
            .transports
            .contains(spider_link::transports::iroh::IROH_SCHEME)
        {
            use spider_link::transports::iroh::IrohConnector;

            if let Some(hub) = &self.iroh_hub {
                let sr = self.state.self_relation.clone();
                let rel = host_relation.clone();

                let connector = IrohConnector::new(sr, rel, hub.endpoint().clone());
                link_set
                    .add_connector(connector)
                    .await
                    .wrap_msg("Failed to add Iroh Connector")?;
            }
        }

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

        self.link_set = Some(link_set);
        Ok(())
    }

    async fn dispose_link_set(&mut self, _link_set: LinkSet<Message>) -> ClientResult {
        // Used to be used to close out Veilid connections
        Ok(())
    }

    async fn run(mut self) -> ClientResult {
        // operate the connection, sending and returning
        let mut connected = false;
        let mut reconnecting = false;

        let mut discovered_addrs:HashMap<Address, u32>  = HashMap::new();
        let mut addr_retry_interval = interval(Duration::from_secs(15));
        loop {
            trace!(
                "connected: {connected}, link_set.is_some() {}, discovery_enable: {}",
                self.link_set.is_some(),
                self.state.discovery_enable
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
                        LinkSetMessage::Disconnected => {
                            connected = false;
                            self.process_client_response(ClientResponse::Disconnected).await;
                        },
                        LinkSetMessage::Connected(epoch) => {
                            connected = true;
                            self.process_client_response(ClientResponse::Connected(epoch)).await;
                        },
                        LinkSetMessage::AttemptingConnection(re_con) => {
                            if re_con{
                                addr_retry_interval.reset_immediately();
                            }
                            reconnecting = re_con;
                        }
                        LinkSetMessage::Message(message, epoch) => {
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
                                self.discoverer = None;
                                discovered_addrs.clear();
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
                                ClientControl::PairAddr(addr) => {
                                    trace!("Pairing to device at {}", addr);
                                    if self.state.host_relation.is_none() {
                                        if let Ok(key_req) = spider_link::transports::tcp::key_request(addr).await{
                                            trace!("Got key request {:?}", key_req);
                                            self.state.host_relation = Some(Relation::peer_from_id(key_req.key));
                                            self.init_link_set().await.wrap_msg("Failed to initialize link set")?;
                                            // add addr to link set to attempt the connection
                                            if let Some(link_set) = self.link_set.as_ref() {
                                                trace!("Adding addr to link_set {}", addr);
                                                let _ = link_set.add_addr( Address::new("auth_tcp",addr.to_string()) ).await;
                                            }else{
                                                // This condition indicates that
                                                // the client could pair but not
                                                // establish a connection for
                                                // whatever reason.
                                            }
                                            let _ = self.save_state().await;
                                            self.process_client_response(ClientResponse::Paired).await;
                                        }

                                    }
                                }
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
                                        }
                                        self.discoverer = None;
                                        // Clear cached base addresses to prevent stale addresses
                                        // from being used when re-pairing
                                        self.state.base_addrs.clear();
                                        discovered_addrs.clear();
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
                addr = opt_next_addr(self.discoverer.as_mut()), if reconnecting && self.link_set.is_some() && self.state.discovery_enable => {
                    info!("Client received addr: {:?}", addr);
                    match addr {
                        AdvertEvent::Found(base_advert) => {
                            if base_advert.id.is_none_or(|id| id == self.state.host_relation.as_ref().unwrap().id) {
                                let addrs = base_advert.addrs;
                                for addr in &addrs{
                                    *discovered_addrs.entry(addr.clone()).or_insert(0) += 1;
                                }
                                // can only add addrs when the link is paired
                                if let Some(link_set) = &self.link_set{
                                    for addr in addrs{
                                        let _ = link_set.try_addr(addr).await;
                                    }
                                }
                            }
                        },
                        AdvertEvent::Lost(base_advert) => {
                            if base_advert.id.is_none_or(|id| id == self.state.host_relation.as_ref().unwrap().id) {
                                for addr in base_advert.addrs{
                                    if let Entry::Occupied(mut e) = discovered_addrs.entry(addr) {
                                        *e.get_mut() -= 1;
                                        if *e.get() == 0 {
                                            e.remove();
                                        }
                                    }
                                }
                            }
                        },
                    }
                }
                _ = addr_retry_interval.tick(), if reconnecting && self.link_set.is_some() && self.state.discovery_enable => {
                    // can only add addrs when the link is paired
                    if let Some(link_set) = &self.link_set{
                        for addr in discovered_addrs.keys(){
                            let _ = link_set.try_addr(addr.clone()).await;
                        }
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
    ls: Option<&mut LinkSet<Message>>,
) -> Result<(&mut LinkSet<Message>, LinkSetMessage<Message>), ClientError> {
    match ls {
        Some(ls) => {
            let msg = ls.recv().await?;
            Ok((ls, msg))
        }
        None => std::future::pending().await,
    }
}

async fn opt_next_addr(d: Option<&mut Box<dyn Discoverer>>) -> AdvertEvent {
    match d {
        Some(d) => d.next_addr().await,
        None => std::future::pending().await,
    }
}

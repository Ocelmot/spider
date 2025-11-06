use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bimap::BiHashMap;
use futures::StreamExt;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender, UnboundedReceiver};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{interval, Interval};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamMap;
use tracing::{info, trace, warn};
use veilid_core::{CryptoKey, CryptoTyped, DHTRecordDescriptor, VeilidAPI, VeilidUpdate};

use crate::error::{ErrorKind, ProblemWrap};
use crate::{LinkError, LinkResult, Relation, SelfRelation};

use super::introduction::VeilidIntroduction;
use super::link::VeilidLink;
use super::pending::{IncomingPending, OutgoingPending};
use super::route_manager::RouteManager;

type DHTKey = CryptoTyped<CryptoKey>;
pub struct VeilidHubInner {
    // Listen components
    listen_rel: SelfRelation,

    // Route Components
    route_manager: RouteManager,

    /// Connection to a Relation using a dht_key
    outgoing_connections: BiHashMap<Relation, DHTKey>,

    /// published dht keys for receiving
    incoming_connections: BiHashMap<DHTKey, Relation>,

    // Pending Connections
    out_pending: HashMap<Relation, OutgoingPending>,
    inc_pending: HashMap<Relation, IncomingPending>,

    update_interval: Interval,

    // Receivers
    connect_rx: Receiver<(Relation, String, oneshot::Sender<VeilidLink>)>,
    update_rx: UnboundedReceiver<VeilidUpdate>,
    /// Set of receivers identified by the destination Relation used by that
    /// connection
    links_rx: StreamMap<Arc<Relation>, ReceiverStream<Vec<u8>>>,

    // Senders
    listener_tx: Sender<VeilidLink>,
    links_tx: HashMap<Relation, Sender<Vec<u8>>>,
}

impl VeilidHubInner {
    pub fn new(
        listen_rel: SelfRelation,
        listen_dht: DHTRecordDescriptor,
        api: VeilidAPI,
        connect_rx: Receiver<(Relation, String, oneshot::Sender<VeilidLink>)>,
        update_rx: UnboundedReceiver<VeilidUpdate>,
        listener_tx: Sender<VeilidLink>,
    ) -> Self {
        let mut update_interval = interval(Duration::from_secs(1));
        update_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self {
            listen_rel,

            route_manager: RouteManager::new(api, listen_dht),

            outgoing_connections: BiHashMap::new(),

            incoming_connections: BiHashMap::new(),

            out_pending: HashMap::new(),
            inc_pending: HashMap::new(),

            update_interval,

            connect_rx,
            update_rx,
            links_rx: StreamMap::new(),
            listener_tx,
            links_tx: HashMap::new(),
        }
    }

    pub fn start(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                let result = select! {
                    Some(update) = self.update_rx.recv(), if !self.update_rx.is_closed() => {
                        self.process_veilid_update(update).await
                    },
                    Some((rel, addr, sender)) = self.connect_rx.recv(), if !self.connect_rx.is_closed() => {
                        self.process_connect_request(rel, addr, sender).await
                    },
                    Some((to, link_msg)) = self.links_rx.next(), if !self.links_rx.is_empty() => {
                        self.process_outgoing_msg(&to, link_msg).await
                    },
                    _ = self.update_interval.tick() => {
                        self.process_update().await
                    }
                    else => {
                        // if all inputs are disabled, close
                        return;
                    }
                };
                if let Err(error) = result {
                    info!("Veilid Hub encountered error: {}", error.print_report());
                }
            }
        })
    }

    async fn process_veilid_update(&mut self, update: VeilidUpdate) -> LinkResult {
        // receive a message from the veilid system, deserialize it, and send it
        // through the matching link, if available, or through the listener
        // otherwise
        match update {
            VeilidUpdate::AppMessage(msg) => {
                let Some(route_id) = msg.route_id() else {
                    warn!("Received Veilid App message without route: {:?}", msg);
                    return Ok(());
                };

                trace!(
                    "Received Veilid AppMessage from {:?}, {} bytes",
                    msg.route_id(),
                    msg.message().len()
                );

                // if coming from listen route_id
                if self.route_manager.is_listen_route(route_id) {
                    trace!("received veilid message on listen route");
                    let data = VecDeque::from(msg.message().to_vec());
                    let intro = VeilidIntroduction::deserialize(data)?;
                    let (rel, is_listen, dht_key) = intro.verify(&self.listen_rel)?;

                    if is_listen {
                        trace!("(Step 2 or 5) Received listen invitation");

                        let inc_dht =
                            if let Some(inc_dht) = self.incoming_connections.get_by_right(&rel) {
                                inc_dht
                            } else {
                                let new_inc_dht = self.route_manager.create_inc_route().await?;
                                self.incoming_connections
                                    .insert(*new_inc_dht.key(), rel.clone());
                                self.incoming_connections
                                    .get_by_right(&rel)
                                    .expect("should have just inserted this element")
                            };

                        let inc_pending = IncomingPending::new(rel.clone(), dht_key, *inc_dht);
                        self.route_manager
                            .send_connection_intro(&self.listen_rel, &inc_pending)
                            .await?;

                        // if a connection does not exist, save incoming pending and request their conn_dht
                        if !self.links_tx.contains_key(&rel)
                        {
                            trace!("(Step 4a) request their connection dht");
                            self.route_manager.send_listen_intro(&self.listen_rel, &inc_pending).await?;
                            self.inc_pending.insert(rel, inc_pending);
                        }
                    } else {
                        trace!("(Step 3 or 6) Received connection dht, emitting link");
                        if let Some(pending) = self.out_pending.remove(&rel) {
                            trace!("(Step 3) found outgoing pending, creating link");
                            let link = self.new_link(rel, dht_key);
                            pending.send(link);
                        } else if let Some(_pending) = self.inc_pending.remove(&rel) {
                            trace!("(Step 6) found incoming pending, creating link");
                            let link = self.new_link(rel, dht_key);
                            self.listener_tx.send(link).await.wrap()?;
                        } else {
                            trace!("Spurious intro, no action");
                        }
                    }

                    return Ok(());
                }

                // if coming from partner route
                if let Some(inc_route) = msg.route_id() {
                    trace!("Received veilid message on partner route {}", inc_route);
                    // get Relation from connection maps
                    if let Some(dht) = self.route_manager.get_inc_dht(inc_route) {
                        if let Some(rel) = self.incoming_connections.get_by_left(dht.key()) {
                            trace!("Sending message over link");
                            // send message to link
                            let link = self.links_tx.get(rel).wrap()?;
                            link.send(msg.message().to_vec()).await.wrap()?;
                        }
                    }
                }
            }
            VeilidUpdate::RouteChange(route_change) => {
                trace!("Received Veilid RouteChange: {:?}", route_change);
                self.route_manager
                    .update_route_changes(route_change)
                    .await?;
            }
            VeilidUpdate::Attachment(attachment) => {
                trace!("Received Veilid Attachment: {:?}", attachment);
                if attachment.as_ref().public_internet_ready {
                    if !self.route_manager.listen_route_initialized() {
                        self.route_manager.initialize_listen_routes().await?;
                    } else {
                    }
                }
            }
            VeilidUpdate::ValueChange(value_change) => {
                trace!("Received Veilid value change {:?}", value_change);
            }
            _ => {}
        }
        Ok(())
    }

    fn new_link(&mut self, rel: Relation, out_dht: DHTKey) -> VeilidLink {
        // update connection items
        // Todo: clear dht entries when an overwrite occurs
        trace!("new link is connected to relation {:?}", rel);
        trace!("new link is will send to DHT {:?}", out_dht);
        self.outgoing_connections.insert(rel.clone(), out_dht);

        let (to_link, from_hub) = channel(25);
        let (to_hub, from_link) = channel(25);

        self.links_tx.insert(rel.clone(), to_link);
        if let Some(mut old_rx) = self
            .links_rx
            .insert(Arc::new(rel.clone()), from_link.into())
        {
            old_rx.close();
        }

        let link = VeilidLink::new(self.listen_rel.clone(), rel, to_hub, from_hub);
        link
    }

    async fn process_connect_request(
        &mut self,
        rel: Relation,
        addr: String,
        sender: oneshot::Sender<VeilidLink>,
    ) -> LinkResult {
        let Ok(listen_dht_key) = CryptoTyped::<CryptoKey>::from_str(&addr) else {
            return Err(LinkError::new()
                .problem(ErrorKind::Deserialization)
                .msg(format!("Failed to parse dht_key from `{}`", addr)));
        };

        // Create new pending
        trace!("(Step 1a) Creating new outgoing pending veilid connection");
        let pending = OutgoingPending::new(
            rel.clone(),
            listen_dht_key,
            *self.route_manager.get_listen_dht_key(),
            sender,
        );
        let _ = self
            .route_manager
            .send_listen_intro(&self.listen_rel, &pending)
            .await;

        // store pending
        trace!("Inserting new outgoing link into pending list");
        self.out_pending.insert(rel, pending);

        Ok(())
    }

    async fn process_outgoing_msg(&mut self, to: &Relation, data: Vec<u8>) -> LinkResult {
        // get dht key
        let Some(dht_key) = self.outgoing_connections.get_by_left(&to) else {
            self.links_rx.remove(to);
            return Ok(());
        };

        self.route_manager.send_msg(dht_key, data).await
    }

    async fn process_update(&mut self) -> LinkResult {
        for (_, pending) in &self.out_pending {
            trace!("(Step 1b) Sending existing outgoing pending");
            let _ = self
                .route_manager
                .send_listen_intro(&self.listen_rel, pending)
                .await;
        }
        for (_, pending) in &self.inc_pending {
            trace!("(Step 4b) Sending existing incoming pending");
            let _ = self
                .route_manager
                .send_listen_intro(&self.listen_rel, pending)
                .await;
        }
        Ok(())
    }
}

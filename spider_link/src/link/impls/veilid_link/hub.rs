use std::sync::Arc;

use tokio::sync::mpsc::{channel, unbounded_channel, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::trace;
use veilid_core::{DHTRecordDescriptor, DHTSchema, DHTSchemaDFLT, VeilidConfigInner};

use crate::error::ProblemWrap;
use crate::{LinkResult, Relation, SelfRelation};

use super::hub_inner::VeilidHubInner;
use super::link::VeilidLink;

/// A VeilidHub manages [Link]s that are backed by the veilid network
pub struct VeilidHub {
    listen_rel: SelfRelation,
    listen_dht: DHTRecordDescriptor,
    connect_tx: Sender<(Relation, String, oneshot::Sender<VeilidLink>)>,
    inner_handle: JoinHandle<()>,
}

impl VeilidHub {
    /// Create a new Veilid hub using the given configuration.
    ///
    /// Returns an instance of itself as well as a listener that will return incoming links
    pub async fn new(
        listen_rel: SelfRelation,
        listen_dht: Option<DHTRecordDescriptor>,
        mut config: VeilidConfigInner,
    ) -> LinkResult<(Self, Receiver<VeilidLink>)> {
        let (update_tx, update_rx) = unbounded_channel();
        let (listener_tx, listener_rx) = channel(25);
        let (connect_tx, connect_rx) = channel(25);

        if config.namespace == "" {
            config.namespace = listen_rel.relation.sha256()[..32].to_string();
        }

        let api = veilid_core::api_startup_config(
            Arc::new(move |update| {
                let _ = update_tx.send(update);
            }),
            config,
        )
        .await
        .wrap()?;

        trace!("Veilid attaching...");
        api.attach().await.wrap()?;

        let listen_dht = if let Some(listen_dht) = listen_dht {
            listen_dht
        } else {
            let schema = DHTSchema::DFLT(DHTSchemaDFLT::new(1).expect("schema should create"));

            let routing = api.routing_context().wrap()?;
            routing.create_dht_record(schema, None, None).await.wrap()?
        };

        let inner = VeilidHubInner::new(listen_rel.clone(), listen_dht.clone(), api, connect_rx, update_rx, listener_tx);

        let inner_handle = inner.start();
        Ok((
            Self {
                listen_rel,
                listen_dht,
                connect_tx,
                inner_handle,
            },
            listener_rx,
        ))
    }

    /// Returns a reference to the relation this hub is listening for
    pub fn listen_rel(&self) -> &SelfRelation {
        &self.listen_rel
    }

    /// Returns a reference to the dht descriptor this hub is receiving on
    pub fn listen_dht(&self) -> &DHTRecordDescriptor {
        &self.listen_dht
    }

    /// Request a link to a node on the veilid network
    pub async fn connect(&self, rel: Relation, addr: String) -> LinkResult<VeilidLink> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.connect_tx.send((rel, addr, reply_tx)).await.wrap()?;
        reply_rx.await.wrap()
    }

    /// Returns a handle that can request connections from the VeilidHub
    pub fn get_connector(&self) -> VeilidConnector {
        VeilidConnector {
            connect_tx: self.connect_tx.clone(),
        }
    }

    /// Stops the hub from processing further information, closing all
    /// connections
    pub fn terminate(self) {
        self.inner_handle.abort();
    }
}

/// A handle to a [VeilidHub] that can request new outgoing connections
#[derive(Clone)]
pub struct VeilidConnector {
    connect_tx: Sender<(Relation, String, oneshot::Sender<VeilidLink>)>,
}

impl VeilidConnector {
    /// Request a link to a node on the veilid network
    pub async fn connect(&self, rel: Relation, addr: String) -> LinkResult<VeilidLink> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.connect_tx.send((rel, addr, reply_tx)).await.wrap()?;
        reply_rx.await.wrap()
    }
}

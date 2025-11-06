use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{channel, Sender};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::trace;

use crate::{Relation, SelfRelation};

use super::{LinkConnector, LinkSetControl};

enum ConnectorControl {
    AddAddr(String),
    TryAddr(String),
    AddConnector(Arc<LinkConnector>),
}

#[derive(Debug)]
pub(crate) struct Connector {
    tx: Sender<ConnectorControl>,
    handle: JoinHandle<()>,
}

impl Connector {
    pub fn start(
        sr: SelfRelation,
        r: Relation,
        ctrl: Sender<LinkSetControl>,
        // addrs is a list of addresses, and a flag to indicate if they are permanent
        mut addrs: Vec<(String, bool)>,
        mut conns: Vec<Arc<LinkConnector>>,
    ) -> Self {
        let (tx, mut rx) = channel(10);
        let handle = tokio::task::spawn(async move {
            let mut addr_index = 0;
            let mut conn_index = 0;
            trace!("Connector started");
            loop {
                // process rx
                loop {
                    let ctrl = if addrs.len() == 0 || conns.len() == 0 {
                        // wait for rx to have a message and start over
                        trace!("Connector waiting for message, addrs: {}, conns: {}", addrs.len(), conns.len());
                        match rx.recv().await {
                            Some(ctrl) => Some(ctrl),
                            None => return,
                        }
                    } else {
                        trace!("Connector checking for message, addrs: {}, conns: {}", addrs.len(), conns.len());
                        match rx.try_recv() {
                            Ok(ctrl) => Some(ctrl),
                            Err(TryRecvError::Empty) => None,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    };

                    trace!("Got control message: {:?}", ctrl);
                    match ctrl {
                        Some(ConnectorControl::AddAddr(add_addr)) => {
                            for (addr, _) in &addrs {
                                if *addr == add_addr {
                                    // skip addrs we already have in the list
                                    continue;
                                }
                            }
                            addrs.push((add_addr, true))
                        },
                        Some(ConnectorControl::TryAddr(try_addr)) => {
                            for (addr, _) in &addrs {
                                if *addr == try_addr {
                                    // skip addrs we already have in the list
                                    continue;
                                }
                            }
                            addrs.push((try_addr, false))
                        },
                        Some(ConnectorControl::AddConnector(conn)) => conns.push(conn),
                        None => break,
                    }
                }

                // process for each addr, each conn
                if let Some((addr, retain_addr)) = addrs.get(addr_index) {
                    if let Some(conn) = conns.get(conn_index) {
                        trace!("Connector attempting to connect to {addr} with connector index {conn_index}");
                        let res = conn(sr.clone(), r.clone(), addr.clone()).await;
                        match res {
                            Ok(link) => {
                                trace!("Connector made connection, adding link");
                                let _ = ctrl.send(LinkSetControl::AddLink(link)).await;
                                return;
                            }
                            Err(e) => {
                                // if there is an error, we will just try again later or with another connector
                                trace!("Connector did not make a connection: {}", e);
                            } 
                        }
                        conn_index += 1;
                    } else {
                        conn_index = 0;
                        if *retain_addr {
                            addr_index += 1;
                        } else {
                            addrs.remove(addr_index);
                        }
                    }
                } else {
                    trace!("Address/Connector list finished, sleeping");
                    addr_index = 0;
                    sleep(Duration::from_secs(5)).await;
                }
            }
        });

        Self { tx, handle }
    }

    pub async fn add_addr(&self, addr: String) {
        let _ = self.tx.send(ConnectorControl::AddAddr(addr)).await;
    }

    pub async fn try_addr(&self, addr: String) {
        let _ = self.tx.send(ConnectorControl::TryAddr(addr)).await;
    }

    pub async fn add_connector(&self, conn: Arc<LinkConnector>) {
        let _ = self.tx.send(ConnectorControl::AddConnector(conn)).await;
    }

    pub fn cancel(self) {
        self.handle.abort();
    }
}

impl Debug for ConnectorControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddAddr(arg0) => f.debug_tuple("AddAddr").field(arg0).finish(),
            Self::TryAddr(arg0) => f.debug_tuple("TryAddr").field(arg0).finish(),
            Self::AddConnector(_) => f.debug_tuple("AddConnector").field(&"<Connector>").finish(),
        }
    }
}
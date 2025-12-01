use std::{mem, sync::Arc};

use tokio::sync::mpsc::Sender;

use crate::{Relation, SelfRelation};

use super::{connector::Connector, LinkConnector, LinkSetControl};

#[derive(Debug)]
pub enum CoreState {
    /// The LinkCore is not connected or trying to connect
    Disconnected,
    /// The LinkCore has disconnected, but is trying to reconnect
    Connecting(Connector),
    /// The LinkCore is experiencing a transient disconnection, and is reconnecting
    Reconnecting(Connector),
    /// The LinkCore is waiting for a potential reconnection
    GracePeriod,
    /// The LinkCore is connected.
    Connected,
}

impl CoreState {
    pub(crate) fn is_connected(&self) -> bool {
        match self {
            CoreState::Connected => true,
            _ => false,
        }
    }

    /// The state is actively trying to move to the connected state.
    pub(crate) fn has_connector(&self) -> bool {
        match self {
            CoreState::Disconnected => false,
            CoreState::Connecting(_) => true,
            CoreState::Reconnecting(_) => true,
            CoreState::GracePeriod => false,
            CoreState::Connected => false,
        }
    }

    /// Move the state to Disconnected. If a connector was running, cancel it.
    /// The return value indicates if the state stopped being connected.
    /// For this purpose, Connecting counts as disconnected, and Reconnecting
    /// counts as connected.
    pub(crate) fn disconnect(&mut self) -> bool {
        let mut other = CoreState::Disconnected;
        mem::swap(self, &mut other);
        match other {
            CoreState::Disconnected => false,
            CoreState::Connecting(connector) => {
                connector.cancel();
                false
            }
            CoreState::Reconnecting(connector) => {
                connector.cancel();
                true
            }
            CoreState::GracePeriod => true,
            CoreState::Connected => true,
        }
    }

    /// Try to start connecting. If the state was disconnected, use the
    /// parameters to create a new connector. All other states continue as they
    /// are. The return value indicates if the state started connecting. If the
    /// state was connected, connecting, and reconnecting, it does not start
    /// connecting.
    pub(crate) fn connect(
        &mut self,
        self_rel: &SelfRelation,
        rel: &Relation,
        ctrl: &Sender<LinkSetControl>,
        addrs: &Vec<String>,
        conns: &Vec<Arc<LinkConnector>>,
    ) -> bool {
        match self {
            CoreState::Disconnected => {
                let self_rel = self_rel.clone();
                let rel = rel.clone();
                let ctrl = ctrl.clone();
                let addrs = addrs.iter().map(|e| (e.clone(), true)).collect();
                let conns: Vec<Arc<LinkConnector>> = conns.clone();
                let connector = Connector::start(self_rel, rel, ctrl, addrs, conns);
                *self = CoreState::Connecting(connector);
                true
            }
            CoreState::Connecting(_) => false,
            CoreState::Reconnecting(_) => false,
            CoreState::GracePeriod => false,
            CoreState::Connected => false,
        }
    }

    /// If the state is Reconnecting, convert to Connecting. All other states remain the same.
    /// The return value indicates if the state transitioned from Reconnecting to Connecting.
    /// If the state is in the grace period, it becomes disconnected instead.
    pub(crate) fn end_transient(&mut self) -> bool {
        // Temporary state while we take ownership and modify
        let mut tmp = CoreState::Disconnected;
        mem::swap(self, &mut tmp);
        let (mut new, ret) = match tmp {
            CoreState::Disconnected => (CoreState::Disconnected, false),
            CoreState::Connecting(connector) => (CoreState::Connecting(connector), false),
            CoreState::Reconnecting(connector) => (CoreState::Connecting(connector), true),
            CoreState::GracePeriod => (CoreState::Disconnected, true),
            CoreState::Connected => (CoreState::Connected, false),
        };
        mem::swap(self, &mut new);
        ret
    }

    /// A transient disconnection has occurred, if state was Connected, start
    /// trying to reconnect. All other states remain the same
    pub(crate) fn start_transient(
        &mut self,
        self_rel: &SelfRelation,
        rel: &Relation,
        ctrl: &Sender<LinkSetControl>,
        addrs: &Vec<String>,
        conns: &Vec<Arc<LinkConnector>>,
    ) {
        // Temporary state while we take ownership and modify
        let mut tmp = CoreState::Disconnected;
        mem::swap(self, &mut tmp);
        let mut new = match tmp {
            CoreState::Disconnected => CoreState::Disconnected,
            CoreState::Connecting(connector) => CoreState::Connecting(connector),
            CoreState::Reconnecting(connector) => CoreState::Reconnecting(connector),
            CoreState::GracePeriod => CoreState::GracePeriod,
            CoreState::Connected => {
                let self_rel = self_rel.clone();
                let rel = rel.clone();
                let ctrl = ctrl.clone();
                let addrs = addrs.iter().map(|e| (e.clone(), true)).collect();
                let conns = conns.clone();
                let connector = Connector::start(self_rel, rel, ctrl, addrs, conns);
                CoreState::Reconnecting(connector)
            }
        };
        mem::swap(self, &mut new);
    }

    /// A transient disconnection has occurred, if state was Connected, start
    /// trying to reconnect. All other states remain the same
    pub(crate) fn start_grace_period(&mut self) {
        // Temporary state while we take ownership and modify
        let mut tmp = CoreState::Disconnected;
        mem::swap(self, &mut tmp);
        let mut new = match tmp {
            CoreState::Disconnected => CoreState::Disconnected,
            CoreState::Connecting(connector) => CoreState::Connecting(connector),
            CoreState::Reconnecting(connector) => CoreState::Reconnecting(connector),
            CoreState::GracePeriod => CoreState::GracePeriod,
            CoreState::Connected => CoreState::GracePeriod,
        };
        mem::swap(self, &mut new);
    }

    /// The state is now connected. if the previous state had a connector, the connector is stopped
    /// The return value indicates if the state started being connected.
    /// For this purpose, Connecting counts as disconnected, and Reconnecting
    /// counts as connected.
    pub(crate) fn connected(&mut self) -> bool {
        let mut other = CoreState::Connected;
        mem::swap(self, &mut other);
        match other {
            CoreState::Disconnected => true,
            CoreState::Connecting(connector) => {
                connector.cancel();
                true
            }
            CoreState::Reconnecting(connector) => {
                connector.cancel();
                false
            }
            CoreState::GracePeriod => false,
            CoreState::Connected => false,
        }
    }

    /// Add an address to the connector after the fact, if the state is
    /// connecting or reconnecting.
    pub(crate) async fn add_addr(&mut self, addr: &String) {
        match self {
            CoreState::Disconnected => {}
            CoreState::Connecting(connector) => {
                connector.add_addr(addr.clone()).await;
            }
            CoreState::Reconnecting(connector) => {
                connector.add_addr(addr.clone()).await;
            }
            CoreState::GracePeriod => {}
            CoreState::Connected => {}
        }
    }

    pub(crate) async fn try_addr(&mut self, addr: &String) {
        match self {
            CoreState::Disconnected => {}
            CoreState::Connecting(connector) => {
                connector.try_addr(addr.clone()).await;
            }
            CoreState::Reconnecting(connector) => {
                connector.try_addr(addr.clone()).await;
            }
            CoreState::GracePeriod => {}
            CoreState::Connected => {}
        }
    }

    /// Add a connector to the connector after the fact, if the state is
    /// connecting or reconnecting.
    pub(crate) async fn add_connector(&mut self, conn: &Arc<LinkConnector>) {
        match self {
            CoreState::Disconnected => {}
            CoreState::Connecting(connector) => {
                connector.add_connector(conn.clone()).await;
            }
            CoreState::Reconnecting(connector) => {
                connector.add_connector(conn.clone()).await;
            }
            CoreState::GracePeriod => {}
            CoreState::Connected => {}
        }
    }
}

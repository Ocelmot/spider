//! The Beacon functionality allows peripherals to find a base on the
//! local network by broadcasting a probe. The response allows the
//! peripheral to find the address of the base.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use futures::{
    stream::{self, SelectAll},
    Stream, StreamExt,
};

use network_interface::NetworkInterfaceConfig;
use tokio::{
    net::UdpSocket,
    select,
    task::JoinHandle,
    time::{interval, timeout, Instant, Interval},
};
use tokio_stream::StreamMap;
use tracing::{error, info, trace, warn};

/// Get a list of UdpSockets connected to each interface, if the interface as a
/// private or loopback address.
async fn get_interface_sockets() -> Vec<UdpSocket> {
    let interfaces = network_interface::NetworkInterface::show().unwrap_or(Vec::new());
    let mut sockets = Vec::new();

    let addrs = interfaces
        .iter()
        .flat_map(|interface| interface.addr.iter());

    for addr in addrs {
        trace!("Beacon binding on {:?}", addr.ip());
        match addr {
            network_interface::Addr::V4(addr) => {
                if addr.ip.is_private() || addr.ip.is_loopback() {
                    match UdpSocket::bind((addr.ip, 1929)).await {
                        Ok(socket) => {
                            if let Err(e) = socket.set_broadcast(true) {
                                warn!("Cant broadcast on {}, due to error {}", addr.ip, e);
                                continue;
                            }

                            sockets.push(socket);
                        }
                        Err(e) => {
                            warn!("Cant bind to {}, due to error {}", addr.ip, e);
                        }
                    }
                }
            }
            network_interface::Addr::V6(addr) => {
                if addr.ip.is_loopback() {
                    match UdpSocket::bind((addr.ip, 1929)).await {
                        Ok(socket) => {
                            if let Err(e) = socket.set_broadcast(true) {
                                warn!("Cant broadcast on {}, due to error {}", addr.ip, e);
                                continue;
                            }

                            sockets.push(socket);
                        }
                        Err(e) => {
                            warn!("Cant bind to {}, due to error {}", addr.ip, e);
                        }
                    }
                }
            }
        }
    }

    sockets
}

async fn beacon_probe_send(sockets: &Vec<UdpSocket>) {
    for socket in sockets {
        match socket.local_addr() {
            Ok(addr) => {
                trace!("Probing for spiders on {} ...", addr);
            }
            Err(_) => {
                trace!("Probing for spiders on Unknown ...",);
            }
        }

        let _ = socket
            .send_to(b"SPIDER_PROBE", "255.255.255.255:1930")
            .await;
    }
}

fn sockets_to_recv_stream(sockets: Vec<UdpSocket>) -> SelectAll<impl Stream<Item = SocketAddr>> {
    let mut ret = SelectAll::new();
    for socket in sockets.into_iter() {
        ret.push(Box::pin(stream::unfold(socket, |socket| async {
            beacon_response_recv(&socket)
                .await
                .map(|addr| (addr, socket))
        })));
    }
    ret
}

/// Beacon broadcasts a request for nearby bases to reply. This yields those
/// replies in an asynchronous way.
pub struct Beacon {
    port: u16,
    listeners: StreamMap<IpAddr, Pin<Box<dyn Stream<Item = SocketAddr> + Send + Sync>>>,
    senders: HashMap<IpAddr, Arc<UdpSocket>>,
    interval: Interval,
}

impl Beacon {
    /// Create a new beacon to query for nearby bases
    pub fn new(period: Duration) -> Self {
        trace!("Launching beacon");
        let mut interval = interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self {
            port: 1930,
            listeners: StreamMap::new(),
            senders: HashMap::new(),
            interval,
        }
    }

    /// Sets the port the beacon will operate on. This is typically 1930. This
    /// is the port the broadcast will be sent to. The broadcast will come from
    /// the port below this one.
    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    /// Clears the sockets from the internal structures, allowing other systems
    /// to bind to them.
    pub fn clear_sockets(&mut self) {
        self.listeners.clear();
        self.senders.clear();
    }

    async fn fill_sockets(&mut self) {
        let interfaces = network_interface::NetworkInterface::show().unwrap_or(Vec::new());

        let addrs = interfaces
            .iter()
            .flat_map(|interface| interface.addr.iter());

        for addr in addrs {
            // skip if we already have a socket
            if self.listeners.contains_key(&addr.ip()) {
                trace!("Addr {} already in beacon set", addr.ip());
                continue;
            }
            // skip if not a private or loopback address
            match addr {
                network_interface::Addr::V4(v4_if_addr) => {
                    if !v4_if_addr.ip.is_private() && !v4_if_addr.ip.is_loopback() {
                        continue;
                    }
                }
                network_interface::Addr::V6(v6_if_addr) => {
                    if !v6_if_addr.ip.is_loopback() {
                        continue;
                    }
                }
            }

            let port = self.port - 1;
            trace!("Beacon binding on {:?}:{}", addr.ip(), port);
            let socket = match UdpSocket::bind((addr.ip(), port)).await {
                Ok(socket) => Arc::new(socket),
                Err(e) => {
                    warn!("Cant bind to {}:{}, due to error {}", addr.ip(), port, e);
                    continue;
                }
            };
            if let Err(e) = socket.set_broadcast(true) {
                warn!("Cant broadcast on {}, due to error {}", addr.ip(), e);
                continue;
            }

            self.senders.insert(addr.ip(), socket.clone());
            self.listeners.insert(
                addr.ip(),
                Box::pin(stream::unfold(socket, |socket| async {
                    beacon_response_recv(&socket)
                        .await
                        .map(|addr| (addr, socket))
                })),
            );
        }
    }

    /// Return the next address received from the beacon
    pub async fn next_addr(&mut self) -> SocketAddr {
        loop {
            trace!("listeners count: {}", self.listeners.len());
            select! {
                biased;
                Some((_, sock_addr)) = self.listeners.next(), if !self.listeners.is_empty() => {
                    return sock_addr;
                }
                _ = self.interval.tick() => {
                    trace!("Beacon ticked");
                    self.fill_sockets().await;
                    trace!("senders count: {}", self.senders.len());
                    for (_, socket) in &self.senders {
                        match socket.local_addr() {
                            Ok(addr) => {
                                trace!("Probing for spiders on {} ...", addr);
                            }
                            Err(_) => {
                                trace!("Probing for spiders on Unknown ...",);
                            }
                        }

                        let _ = socket
                            .send_to(b"SPIDER_PROBE", ("255.255.255.255", self.port))
                            .await;
                    }
                }
            }
        }
    }
}

async fn beacon_response_recv(socket: &UdpSocket) -> Option<SocketAddr> {
    let mut buf = [0; 1024];
    loop {
        let recv_res = socket.recv_from(&mut buf).await;
        if let Err(e) = &recv_res {
            error!("beacon send error: {}", e);
        }

        let (size, from) = recv_res.ok()?;

        trace!("probe received: {} bytes from {}", size, from);
        let msg = &mut buf[..size];
        let msg_txt = String::from_utf8_lossy(&msg);
        trace!("probe received: {}", msg_txt);

        let parts = msg_txt.split(':').collect::<Vec<_>>();
        if parts.len() < 2 {
            continue;
        }

        if parts[0] == "SPIDER_REPLY" {
            let port = match parts[1..].join(":").parse::<u16>() {
                Ok(port) => port,
                Err(_) => continue,
            };
            let mut to = from.clone();
            to.set_port(port);
            break Some(to);
        }
    }
}

/// Starts the beacon handler that will respond with the given port number. The
/// typical value for the Spider application is 1930.
///
/// The address portion of the beacon is pulled from the udp response. The
/// return value is the JoinHandle for the loop, which can be used to cancel the
/// beacon handler.
pub fn start_beacon_listen_handler(advert_port: u16) -> JoinHandle<()> {
    start_beacon_listen_handler_on(advert_port, 1930u16)
}

/// Starts the beacon handler that will respond with the given port number. The
/// typical value for the Spider application is 1930.
///
/// The beacon typically listens at 1930, but this allows an override to listen
/// on any port.
///
/// The address portion of the beacon is pulled from the udp response. The
/// return value is the JoinHandle for the loop, which can be used to cancel the
/// beacon handler.
pub fn start_beacon_listen_handler_on(advert_port: u16, listen_port: u16) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = [0; 1024];

        info!("Beacon binding on port {}", listen_port);
        let socket = UdpSocket::bind(("0.0.0.0", listen_port)).await.unwrap();
        loop {
            trace!("probe looping");
            let (size, from) = socket.recv_from(&mut buf).await.unwrap();

            info!("probe received: {} bytes from {}", size, from);
            let msg = &mut buf[..size];
            let msg_txt = String::from_utf8_lossy(&msg);
            info!("probe received: {}", msg_txt);

            if msg == b"SPIDER_PROBE" {
                let addr = from;
                info! {"sending reply to {}", addr};
                // it isn't always clear what the address of this device is
                // if it is listening on 0.0.0.0.
                // let the other side get the address from the reply, but send
                // the port number to connect to.
                let reply = format!("SPIDER_REPLY:{}", advert_port);
                socket
                    .send_to(&reply.as_bytes().to_vec(), addr)
                    .await
                    .unwrap();
            }
        }
    })
}
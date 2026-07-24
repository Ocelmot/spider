//! The Beacon functionality allows peripherals to find a base on the
//! local network by broadcasting a probe. The response allows the
//! peripheral to find the address of the base.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use futures::{
    stream::{self},
    Stream, StreamExt,
};

use link_set::links::Address;
use network_interface::{Addr, NetworkInterfaceConfig};
use tokio::{
    net::UdpSocket,
    select,
    sync::watch,
    task::JoinHandle,
    time::{interval, sleep, Interval},
};
use tokio_stream::StreamMap;
use tracing::{debug, error, info, trace, warn};

use crate::{
    discovery::{AdvertEvent, BaseAdvert, Discoverer},
    error::ErrorKind,
    transports::tcp::TCP_SCHEME,
    LinkResult,
};

const PROBE_INTRO: &'static [u8] = b"SPDRPRB1";
const REPLY_INTRO: &'static [u8] = b"SPDRPLY1";

/// Beacon broadcasts a request for nearby bases to reply. This yields those
/// replies in an asynchronous way.
pub struct Beacon {
    port: u16,
    listeners: StreamMap<Addr, Pin<Box<dyn Stream<Item = BaseAdvert> + Send + Sync>>>,
    senders: HashMap<Addr, Arc<UdpSocket>>,
    interval: Interval,
    seen_adverts: HashMap<BaseAdvert, u8>,
    lost_adverts: HashSet<BaseAdvert>,
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
            seen_adverts: HashMap::new(),
            lost_adverts: HashSet::new(),
        }
    }

    /// Sets the port the beacon will operate on. This is typically 1930. This
    /// is the port the broadcast will be sent to.
    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    async fn fill_sockets(&mut self) {
        let interfaces = network_interface::NetworkInterface::show().unwrap_or(Vec::new());

        let addrs: HashSet<&Addr> = interfaces
            .iter()
            .flat_map(|interface| interface.addr.iter())
            .collect();

        // Prune senders/listeners to remove addrs that are no longer available
        self.senders.retain(|addr, _| addrs.contains(addr));
        let stale_addrs: Vec<Addr> = self
            .listeners
            .keys()
            .filter(|addr| !addrs.contains(addr))
            .copied()
            .collect();
        for addr in stale_addrs {
            self.listeners.remove(&addr);
        }

        for addr in addrs {
            // skip if we already have a socket
            if self.listeners.contains_key(&addr) {
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

            trace!("Beacon binding on {:?}", addr.ip());
            let socket = match UdpSocket::bind((addr.ip(), 0)).await {
                Ok(socket) => Arc::new(socket),
                Err(e) => {
                    warn!("Cant bind to {}, due to error {}", addr.ip(), e);
                    continue;
                }
            };
            if let Err(e) = socket.set_broadcast(true) {
                warn!("Cant broadcast on {}, due to error {}", addr.ip(), e);
                continue;
            }

            self.senders.insert(addr.clone(), socket.clone());
            self.listeners.insert(
                addr.clone(),
                Box::pin(stream::unfold(socket, |socket| async {
                    beacon_response_recv(&socket)
                        .await
                        .map(|advert| (advert, socket))
                })),
            );
        }
    }
}

impl Discoverer for Beacon {
    fn next_addr(&mut self) -> Pin<Box<dyn Future<Output = AdvertEvent> + Send + '_>> {
        let fut = async {
            loop {
                trace!("listeners count: {}", self.listeners.len());

                select! {
                    biased;
                    Some((_addr, advert)) = self.listeners.next(), if !self.listeners.is_empty() => {
                        // if we see a new advert for an item in the lost adverts, cancel the lost
                        let rescued = self.lost_adverts.remove(&advert);

                        // if new advert is in our set, update the count
                        if let Some(count) = self.seen_adverts.get_mut(&advert) {
                            *count = 3;
                        }else{
                            // new advert
                            self.seen_adverts.insert(advert.clone(), 3);
                            if !rescued {
                                return AdvertEvent::Found(advert);
                            }
                        }
                    }
                    _ = std::future::ready(()), if !self.lost_adverts.is_empty() => {
                        let lost = self.lost_adverts.extract_if(|_|true).next();
                        if let Some(lost) = lost{
                            return AdvertEvent::Lost(lost);
                        }
                    }
                    _ = self.interval.tick() => {
                        trace!("Beacon ticked");
                        let lost = self.seen_adverts.extract_if(|_advert, count|{
                            *count = count.saturating_sub(1);
                            *count == 0
                        }).map(|(advert, _count)|advert);
                        for advert in lost{
                            self.lost_adverts.insert(advert);
                        }

                        self.fill_sockets().await;
                        trace!("senders count: {}", self.senders.len());
                        for (if_addr, socket) in &self.senders {
                            match socket.local_addr() {
                                Ok(addr) => {
                                    trace!("Probing for spiders on {} ...", addr);
                                }
                                Err(_) => {
                                    trace!("Probing for spiders on Unknown ...",);
                                }
                            }

                            let broadcast_addr = match if_addr {
                                Addr::V4(v4_if_addr) => match v4_if_addr.broadcast{
                                    Some(broadcast) => IpAddr::V4(broadcast),
                                    None => IpAddr::V4(v4_if_addr.ip),
                                },
                                Addr::V6(v6_if_addr) => IpAddr::V6(v6_if_addr.ip),
                            };

                            let _ = socket
                                .send_to(PROBE_INTRO, SocketAddr::new(broadcast_addr, self.port))
                                .await;
                        }
                    }
                }
            }
        };

        Box::pin(fut)
    }
}

async fn beacon_response_recv(socket: &UdpSocket) -> Option<BaseAdvert> {
    let mut buf = [0; 1024];
    let mut failures = 0;
    loop {
        let (size, from) = match socket.recv_from(&mut buf).await {
            Ok(val) => {
                failures = 0;
                val
            }
            Err(e) => {
                error!("Failed to recv beacon: {}", e);
                failures += 1;
                if failures > 5 {
                    break None;
                } else {
                    sleep(Duration::from_secs(2)).await;
                    continue;
                }
            }
        };

        trace!("probe received: {} bytes from {}", size, from);
        let mut msg = &buf[..size];

        let attempt = (|| {
            let intro = msg
                .split_off(..REPLY_INTRO.len())
                .ok_or(ErrorKind::Deserialization)?;
            if intro != REPLY_INTRO {
                Err(ErrorKind::Deserialization)?;
            }

            let mut advert = BaseAdvert::from_bytes(msg)?;

            for addr in &mut advert.addrs {
                if addr.scheme() != TCP_SCHEME {
                    continue;
                }

                let Ok(mut sock) = SocketAddr::try_from(addr.addr()) else {
                    continue;
                };
                // If addr is unspecified, use the addr it came from
                if sock.ip().is_unspecified() {
                    sock.set_ip(from.ip());
                }

                *addr = Address::new(addr.scheme(), sock);
            }

            LinkResult::Ok(advert)
        })();

        match attempt {
            Ok(advert) => {
                if advert.is_empty() {
                    debug!("got empty advert");
                    continue;
                }
                break Some(advert);
            }
            Err(e) => debug!("Beacon parse failed on {:?} with msg {e}", msg),
        }
    }
}

/// Starts the beacon handler that will respond with the given port number. The
/// typical value for the Spider application is 1930.
///
/// The address portion of the beacon is pulled from the udp response. The
/// return value is the JoinHandle for the loop, which can be used to cancel the
/// beacon handler.
pub fn start_beacon_listen_handler(
    template: watch::Receiver<BaseAdvert>,
    advert_port: u16,
) -> JoinHandle<()> {
    start_beacon_listen_handler_on(template, advert_port, 1930u16)
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
pub fn start_beacon_listen_handler_on(
    template: watch::Receiver<BaseAdvert>,
    advert_port: u16,
    listen_port: u16,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = [0; 1024];

        info!("Beacon binding on port {}", listen_port);
        let socket = UdpSocket::bind(("0.0.0.0", listen_port)).await.unwrap();
        loop {
            trace!("probe looping");
            let (size, from) = match socket.recv_from(&mut buf).await {
                Ok(val) => val,
                Err(e) => {
                    warn!("Beacon listener failed to read from socket: {e}");
                    sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };

            info!("probe received: {} bytes from {}", size, from);
            let msg = &mut buf[..size];
            let msg_txt = String::from_utf8_lossy(&msg);
            info!("probe received: {}", msg_txt);

            if msg == PROBE_INTRO {
                let addr = from;
                info! {"sending reply to {}", addr};

                // Build the reply
                let mut reply = Vec::new();
                reply.extend_from_slice(REPLY_INTRO);
                // Since this device doesn't know which addr its sending over,
                // let the peer deduce our address from the packet it gets.
                // Since a UNSPECIFIED addr doesn't make sense, the receiver can
                // replace with the addr they saw

                let address = Address::new(
                    TCP_SCHEME,
                    SocketAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, advert_port)),
                );

                let mut advert = template.borrow().clone();
                advert.addrs.push(address);
                reply.extend_from_slice(&advert.to_bytes());

                let _ = socket.send_to(&reply, addr).await;
            }
        }
    })
}

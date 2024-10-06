//! The Beacon functionality allows peripherals to find a base on the
//! local network by broadcasting a probe. The response allows the
//! peripheral to find the address of the base.

use std::{net::SocketAddr, time::Duration};

use futures::{
    stream::{self, SelectAll},
    Stream, StreamExt,
};

use log::info;
use network_interface::NetworkInterfaceConfig;
use tokio::{
    net::UdpSocket,
    time::{timeout, Instant},
};

/// Broadcast a request over the local network for any base that is
/// listening. The IP address of the first response received is returned.
/// This function will timeout after 10 seconds.
pub async fn beacon_lookout_one(limit: Duration) -> Option<String> {
    let sockets = get_interface_sockets().await;
    beacon_probe_send(&sockets).await;
    let mut recv_stream = sockets_to_recv_stream(sockets);

    let start = Instant::now();
    let remaining = limit.saturating_sub(start.elapsed());
    while remaining > Duration::ZERO {
        if let Ok(Some(socket_addr)) = timeout(remaining, recv_stream.next()).await {
            return Some(socket_addr.to_string());
        } else {
            break;
        };
    }
    None
}

/// Broadcast a request over the local network for any base that is
/// listening. Returns a Vec of the IP addresses of the responses
/// received during the time limit.
/// This function will timeout after the given Duration.
pub async fn beacon_lookout_many(limit: Duration) -> Vec<String> {
    let sockets = get_interface_sockets().await;
    beacon_probe_send(&sockets).await;
    let mut recv_stream = sockets_to_recv_stream(sockets);

    let start = Instant::now();
    let mut remaining = limit.saturating_sub(start.elapsed());
    let mut addrs = Vec::new();
    while remaining > Duration::ZERO {
        if let Ok(Some(socket_addr)) = timeout(remaining, recv_stream.next()).await {
            addrs.push(socket_addr.to_string());
        } else {
            break;
        };

        remaining = limit.saturating_sub(start.elapsed());
    }

    addrs
}

/// Get a list of UdpSockets connected to each interface, if the interface as a
/// private or loopback address.
async fn get_interface_sockets() -> Vec<UdpSocket> {
    let interfaces = network_interface::NetworkInterface::show().unwrap_or(Vec::new());
    let mut sockets = Vec::new();

    let addrs = interfaces
        .iter()
        .flat_map(|interface| interface.addr.iter());

    for addr in addrs {
        match addr {
            network_interface::Addr::V4(addr) => {
                if addr.ip.is_private() || addr.ip.is_loopback() {
                    let socket = UdpSocket::bind((addr.ip, 1929)).await.unwrap();
                    socket.set_broadcast(true);

                    sockets.push(socket);
                }
            }
            network_interface::Addr::V6(addr) => {
                if addr.ip.is_loopback() {
                    let socket = UdpSocket::bind((addr.ip, 1929)).await.unwrap();
                    socket.set_broadcast(true);

                    sockets.push(socket);
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
                info!("Probing for spiders on {} ...", addr);
            },
            Err(_) => {
                info!("Probing for spiders on Unknown ...", );
            },
        }
        
        socket
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

async fn beacon_response_recv(socket: &UdpSocket) -> Option<SocketAddr> {
    let mut buf = [0; 1024];
    loop {
        let (size, from) = socket.recv_from(&mut buf).await.ok()?;

        info!("probe received: {} bytes from {}", size, from);
        let msg = &mut buf[..size];
        let msg_txt = String::from_utf8_lossy(&msg);
        info!("probe received: {}", msg_txt);

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

//! The Beacon functionality allows peripherals to find a base on the
//! local network by broadcasting a probe. The response allows the
//! peripheral to find the address of the base.

use std::time::Duration;

use tokio::{
    net::UdpSocket,
    time::{timeout, Instant},
};

/// Broadcast a request over the local network for any base that is
/// listening. The IP address of the first response recieved is returned.
/// This function will timeout after 10 seconds.
pub async fn beacon_lookout_one() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:1929").await.unwrap();
    beacon_probe_send(&socket).await;

    let start = Instant::now();
    let limit = Duration::from_secs(10);
    let mut remaining = limit.saturating_sub(start.elapsed());
    while remaining > Duration::ZERO {
        let res = beacon_response_recv(&socket, remaining).await;
        if res.is_some() {
            return res;
        }
        remaining = limit.saturating_sub(start.elapsed());
    }
    None
}

/// Broadcast a request over the local network for any base that is
/// listening. Returns a Vec of the IP addresses of the responses
/// recieved during the time limit.
/// This function will timeout after the given Duration.
pub async fn beacon_lookout_many(limit: Duration) -> Vec<String> {
    let socket = UdpSocket::bind("0.0.0.0:1929").await.unwrap();
    beacon_probe_send(&socket).await;

    let start = Instant::now();
    let mut remaining = limit.saturating_sub(start.elapsed());
    let mut addrs = Vec::new();
    while remaining > Duration::ZERO {
        let res = beacon_response_recv(&socket, remaining).await;
        if let Some(addr) = res {
            addrs.push(addr);
        }
        remaining = limit.saturating_sub(start.elapsed());
    }
    addrs
}

async fn beacon_probe_send(socket: &UdpSocket) {
    socket.set_broadcast(true);
    println!("Probing for spiders...");
    socket
        .send_to(b"SPIDER_PROBE", "255.255.255.255:1930")
        .await;
}

async fn beacon_response_recv(socket: &UdpSocket, duration: Duration) -> Option<String> {
    let mut buf = [0; 1024];
    let x = timeout(duration, async {
        loop {
            let (size, from) = socket.recv_from(&mut buf).await.unwrap();

            println!("probe recieved: {} bytes from {}", size, from);
            let msg = &mut buf[..size];
            let msg_txt = String::from_utf8_lossy(&msg);
            println!("probe recieved: {}", msg_txt);

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
                break Some(to.to_string());
            }
            break None;
        }
    })
    .await;
    x.ok().flatten()
}

use std::{net::SocketAddr, str::FromStr};

use tokio::{net::UdpSocket, task::JoinHandle};

use crate::config::SpiderConfig;

pub(crate) fn start_beacon(config: &SpiderConfig) -> JoinHandle<()> {
    let listen_addr = config.listen_addr.clone();
    let port = match SocketAddr::from_str(&listen_addr) {
        Ok(addr) => addr.port(),
        Err(_) => 1930u16,
    };
    tokio::spawn(async move {
        let mut buf = [0; 1024];

        let socket = UdpSocket::bind("0.0.0.0:1930").await.unwrap();
        // socket.
        loop {
            println!("probe looping");
            let (size, from) = socket.recv_from(&mut buf).await.unwrap();

            println!("probe recieved: {} bytes from {}", size, from);
            let msg = &mut buf[..size];
            let msg_txt = String::from_utf8_lossy(&msg);
            println!("probe recieved: {}", msg_txt);

            if msg == b"SPIDER_PROBE" {
                let addr = from;
                println! {"sending reply to {}", addr};
                // it isnt always clear what the address of this device is,
                // if it is listening on 0.0.0.0.
                // let the other side get the address from the reply, but send
                // the port number to connect to.
                let reply = format!("SPIDER_REPLY:{}", port);
                socket
                    .send_to(&reply.as_bytes().to_vec(), addr)
                    .await
                    .unwrap();
            }
        }
    })
}

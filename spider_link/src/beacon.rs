use std::time::Duration;

use tokio::{net::UdpSocket, time::timeout};




pub async fn beacon_lookout() -> Option<String>{

    let mut buf = [0; 1024];

        let socket = UdpSocket::bind("0.0.0.0:1932").await.unwrap();
        socket.set_broadcast(true);
        let tries = 3;
        for _ in 0..tries {
            println!("Probing for spiders...");
            socket.send_to(b"SPIDER_PROBE", "255.255.255.255:1931").await;
            
            let res = timeout(Duration::from_secs(5), async{
                loop{
                    let (size, from) = socket.recv_from(& mut buf).await.unwrap();
    
                    println!("probe recieved: {} bytes from {}", size, from);
                    let msg = &mut buf[..size];
                    let msg_txt = String::from_utf8_lossy(&msg);
                    println!("probe recieved: {}", msg_txt);
                    
                    let parts = msg_txt.split(':').collect::<Vec<_>>();
                    if parts.len() < 2 {
                        continue;
                    }

                    if parts[0] == "SPIDER_REPLY" {
                        let to = parts[1..].join(":");
                        break Some(to);
                    }
                    break None;
                }

            }).await;

            match res {
                Ok(Some(addr)) => {
                    return Some(addr);
                },
                _ => {}
            }
            
        }


    None
}
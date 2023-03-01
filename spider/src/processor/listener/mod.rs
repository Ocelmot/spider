use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use crate::processor::ProcessorMessage;
use crate::{config::SpiderConfig, state_data::StateData};
use spider_link::link::Link;

use super::router::RouterProcessorMessage;
use super::sender::ProcessorSender;

pub struct Listener {
    beacon: JoinHandle<()>,
    acceptor: JoinHandle<()>,
}

impl Listener {
    pub(crate) fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender) -> Self {
        // start beacon
        let beacon = start_beacon(&config);
        // start listen accept loop to create enw connections and send them through the pipe.
        let acceptor = start_listener(&config, &state, sender);

        // if the pipe is dead, shut down by exiting the loop and closing the beacon

        Self { beacon, acceptor }
    }
}

fn start_beacon(config: &SpiderConfig) -> JoinHandle<()> {
    let listen_addr = config.listen_addr.clone();
    tokio::spawn(async move {
        let mut buf = [0; 1024];

        let socket = UdpSocket::bind("0.0.0.0:1931").await.unwrap();
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
                // let reply = format!("SPIDER_REPLY:{}", listen_addr);
                // we dont always know what the address of this device is, if it is bound to 0.0.0.0
                // let the other side surmise the address from the reply
                let reply = format!("SPIDER_REPLY:");
                socket
                    .send_to(&reply.as_bytes().to_vec(), addr)
                    .await
                    .unwrap();
            }
        }
    })
}

fn start_listener(
    config: &SpiderConfig,
    state_data: &StateData,
    sender: ProcessorSender,
) -> JoinHandle<()> {
    let state_data = state_data.clone();
    let listen_addr = config.listen_addr.clone();
    tokio::spawn(async move {
        let self_relation = state_data.self_relation().await;
        let mut listener = Link::listen(self_relation, listen_addr);
        loop {
            match listener.recv().await {
                None => {
                    break; // no new link, listener is closed
                }
                Some(link) => {
                    let msg = RouterProcessorMessage::NewLink(link);
                    match sender.send(ProcessorMessage::RouterMessage(msg)).await {
                        Ok(_) => {}
                        Err(_) => {
                            break; // channel is closed, processor must also be closed.
                        }
                    }
                }
            }
        }
    })
}

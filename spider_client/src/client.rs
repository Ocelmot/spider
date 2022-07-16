use dht_chord::{TCPAdaptor, adaptor::{ChordAdaptor, AssociateClient}};
use serde::{Serialize, Deserialize};
use tracing::{error, info};

use std::{path::{Path, self}, io::ErrorKind, time::Duration};

use serde_json::{Deserializer, error::Category};

use tokio::{task::JoinHandle, net::{TcpStream, UdpSocket}, io::{AsyncReadExt, AsyncWriteExt}, time::timeout, sync::mpsc::{Sender, Receiver, channel}, select};

use crate::SpiderClientConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message{
    Introduction{id: u32, as_peripheral: bool},
    Message{msg_type: String, routing: Option<u32>, body: Vec<u8>},
}


pub struct SpiderClient{
    config: SpiderClientConfig,


}


impl SpiderClient {

    pub fn new() -> Self {
        SpiderClient{
            config: SpiderClientConfig::new(),
        }
    }

    pub fn from_config(config: SpiderClientConfig) -> Self {
        SpiderClient{
            config,
        }
    }

    pub fn from_file(path: &Path) -> Self {
        let config = SpiderClientConfig::from_file(path);
        Self::from_config(config)
    }

    pub async fn start(self) -> SpiderClientHandle {
        // find address
        let address = self.find_address().await.unwrap();
        let (to_tx, to_rx) = channel(50);
        let (from_tx, from_rx) = channel(50);
        // connect to address
        // start connection handler task
        info!("Connecting to address: {}", address);
        let stream = TcpStream::connect(address).await;
        let handle = match stream {
            Ok(stream) => {
                info!("Connected!");
                to_tx.send(Message::Introduction { id: self.config.node_id, as_peripheral: true }).await;
                SpiderClient::connection_splice(stream, from_tx, to_rx)
            },
            Err(e) => {
                error!("Error, failed to connect: {:?}", e);
                panic!()
            },
        };

        SpiderClientHandle{
            handle: Some(handle),
            channel_to: to_tx,
            channel_from: from_rx,
        }
    }

    fn connection_splice(mut stream: TcpStream, from: Sender<Message>, mut to: Receiver<Message>) -> JoinHandle<()>{
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            loop {
                let mut read_buffer = [0; 1024];
                select! {
                    // if recieved data from stream, attempt to deserialize and send to 'from'
                    read_len = stream.read(&mut read_buffer) => {
                        info!("In read");
                        match read_len {
                            Ok(0) => {
                                info!("No more data!");
                                return; // No more data
                            }, 
                            Ok(len) => { // Append data to buffer
                                buffer.extend_from_slice(&read_buffer[..len]);
                            },
                            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                                continue; // try to read again
                            },
                            Err(e) =>{
                                error!("Encountered error reading from connection: {}", e);
                                // probably should terminate connection here, depending on error
                                break;
                            }
                        }
                        // attempt to deserialize buffer
                        let mut deserializer = Deserializer::from_slice(buffer.as_slice()).into_iter();
    
                        // if successful, truncate buffer, return deserialized struct
                        for result in &mut deserializer{
                            info!("Deserialized: {:?}", result);
                            match result{
                                Ok(msg) => {
                                    if let Message::Introduction{..} = msg{
                                        continue;
                                    }
                                    from.send(msg).await;
                                    
                                },
                                Err(ref e) if e.classify() == Category::Eof => {
                                    break; // if we have encountered an EOF, more information may arrive later
                                },
                                Err(e) => {
                                    error!("Encountered deserialization error: {}\n\tDeserialization buffer: {:?}", e, String::from_utf8(buffer.clone()).unwrap());
                                },
                            }
                        }
                        buffer = buffer[deserializer.byte_offset()..].to_vec();
                    },
                    // if recieved message on 'to', serialized and send to stream
                    msg = to.recv() => {
                        info!("In write, sending message: {:?}", msg);
                        let raw_data = serde_json::ser::to_string(&msg).expect("Failed to serialize struct");
                        stream.write(raw_data.as_bytes()).await;
                    }
                }
            }
            info!("Exited main client loop")
        })
    }

    async fn find_address(&self) -> Option<String> {
        if let Some(addr) = &self.config.host_addr_override{
            return Some(addr.clone());
        }

        // check beacon for address
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

        // check chord for address
        let address_list = self.config.addr_list.as_ref();
        if let Some(addr_list) = address_list {
            for addr in addr_list {
                let mut associate: AssociateClient<String, u32> = TCPAdaptor::associate_client(addr.clone());
                if let Some((id, addr)) = associate.successor_of(self.config.node_id).await{
                    if id == self.config.node_id {
                        return Some(addr);
                    }
                }
            }
        }

        None
    }


}

pub struct SpiderClientHandle{
    handle: Option<JoinHandle<()>>,
    channel_to: Sender<Message>,
    channel_from: Receiver<Message>,
}

impl SpiderClientHandle {

    pub async fn recv(&mut self) -> Option<Message>{
        self.channel_from.recv().await
    }

    pub async fn emit(&mut self, msg: Message) {
        self.channel_to.send(msg).await;
    }

    pub async fn join(&mut self){
        match self.handle.take(){
            Some(handle) => {handle.await;},
            None => {},
        }
    }
    
}
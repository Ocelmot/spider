mod config;
use config::SpiderConfig;

mod message;
use message::{SpiderMessage, Message};


use serde_json::{Deserializer, error::Category};
use tokio::{
    task::JoinHandle,
    net::{TcpListener, UdpSocket, TcpStream},
    io::{AsyncWriteExt, AsyncReadExt},
    sync::mpsc::{channel, Sender},
    select
};
use tracing::{info, debug, error, Subscriber};
use tracing_appender::rolling::{Rotation, RollingFileAppender};

use dht_chord::{TCPChord, chord::ChordHandle, associate::AssociateChannel};
use tracing_subscriber::{field::debug, fmt::layer, Layer, filter::{self, filter_fn}, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt};

use std::{env, io::{self, ErrorKind}, path::{Path, PathBuf}, collections::BTreeMap, fmt::format};

use crate::message::Control;


#[tokio::main]
async fn main() -> Result<(), io::Error> {



    // command line arguments: <filename>
    // filename is name of config file, defaults to config.json
    let mut args = env::args().skip(1);
    let path_str = args.next().unwrap_or("spider_config.json".to_string());
    let config_path = Path::new(&path_str);
    let config = SpiderConfig::from_file(&config_path);

    let log_path = config.log_path.clone().unwrap_or(format!("spider.log"));

    // Setup tracing
    let filter = filter_fn(|metadata|{
        metadata.target() == "spider"
    });

    let file_appender = RollingFileAppender::new(Rotation::NEVER, "", log_path);
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let subscriber = tracing_subscriber::fmt()
        .pretty().with_ansi(false)
        .with_writer(non_blocking)
        .finish();
    subscriber.with(filter).init();

    info!("Starting!");
    info!("Loaded config: {:?}", config);

    
    // Start beacon
    let _beacon = start_beacon(&config).await;
    
    // Start and connect Chord
    let handle = start_chord(&config).await;


    // Start main application listener
    let (processor, in_tx) = start_processor(&config, handle.get_associate().await);
    let _listener = start_listener(&config, in_tx).await;
   


    let _ = processor.await;

    Ok(())
}

async fn start_listener(config: &SpiderConfig, channel: Sender<(SpiderId, SpiderMessage)>) -> JoinHandle<()>{
    let self_id = config.id;
    let listen_addr = config.listen_addr.clone();
    tokio::spawn(async move{
        let listener = TcpListener::bind(listen_addr).await.expect("listener should not fail");
        loop{
            match listener.accept().await {
                Err(e) => { 
                    panic!("Encountered an error in accept: {}", e)
                },
                Ok((stream, addr)) => {
                    println!("Accepted connection from: {}", addr);
                    let tx = create_connection(stream, channel.clone());
                    tx.send(Message::Introduction { id: self_id, as_peripheral: false }).await;
                }
            }
        }
    })
}

async fn start_beacon(config: &SpiderConfig) -> JoinHandle<()> {
    let listen_addr = config.listen_addr.clone();
    tokio::spawn(async move{
        let mut buf = [0; 1024];

        let socket = UdpSocket::bind("0.0.0.0:1931").await.unwrap();
        loop{
            println!("probe looping");
            let (size, from) = socket.recv_from(& mut buf).await.unwrap();
    
            println!("probe recieved: {} bytes from {}", size, from);
            let msg = &mut buf[..size];
            let msg_txt = String::from_utf8_lossy(&msg);
            println!("probe recieved: {}", msg_txt);
    
            if msg == b"SPIDER_PROBE" {
                let addr = from;
                // let to = format!("{}:1931", addr);
                println!{"sending reply to {}", addr};
                let reply = format!("SPIDER_REPLY:{}", listen_addr);
                socket.send_to(&reply.as_bytes().to_vec(), addr).await.unwrap();
            }
        }

    })
}

async fn start_chord(config: &SpiderConfig) -> ChordHandle<String, u32>{
    let filepath = PathBuf::from(match &config.chord_file {
        Some(path) => path,
        None => "spider_chord.json",
    });

    let mut chord = if filepath.exists() {
        TCPChord::from_file(filepath.to_path_buf()).await
    } else {
        let mut chord = TCPChord::new(config.listen_addr.clone() , config.id);
        chord.set_file(Some(filepath.to_path_buf()));
        chord
    };
    chord.set_advert(Some(config.pub_addr.as_bytes().to_vec()));
    chord.start(None).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpiderId{
    Local(u32),
    Remote(u32), 
}


fn start_processor(config: &SpiderConfig, mut chord: AssociateChannel<String, u32>) -> (JoinHandle<()>, Sender<(SpiderId, SpiderMessage)>){
    let self_id = config.id;
    let local_ids = config.local_ids.clone();
    let (channel_in, mut channel) = channel::<(SpiderId, SpiderMessage)>(50);
    let channel_ret = channel_in.clone();
    let handle = tokio::spawn(async move{

        // set of connections for local and remote
        let mut local_connections = BTreeMap::<u32, Sender<Message>>::new();
        let mut remote_connections = BTreeMap::<u32, Sender<Message>>::new();

        loop {
            let (from_id, message) = if let Some(message) = channel.recv().await {
                message
            }else{
                return;
            };

            info!("Processing spider message: {:?}", message);

            let message = match message{
                SpiderMessage::Control(control) => {
                    match control{
                        message::Control::Introduction { id, channel } => {
                            match id {
                                SpiderId::Local(id) => {
                                    if local_ids.contains(&id){
                                        local_connections.insert(id, channel);
                                    }
                                },
                                SpiderId::Remote(id) => {
                                    remote_connections.insert(id, channel);
                                },
                            }
                        },
                    }
                    continue;
                },
                SpiderMessage::Message(message) => message,
            };

            let routing = match message{
                // These should not get into this system. Probably needs a better type representation
                Message::Introduction { .. } => {break;}, 
                //
                Message::Message { routing, .. } => {
                    routing
                },
            };
    
            for (conn_id, conn) in &local_connections{
                // Dont send messages back to sender
                if let SpiderId::Local(from_id) = from_id{
                    if from_id == *conn_id {
                        continue;
                    }
                }

                // TODO: should add a filter here for what this connection is subscribed to 
                conn.send(message.clone()).await;
            }
    
            if let Some(to) = routing {
                if to == self_id {
                    continue; // dont route messages that were routed to us
                }
                // two possibilities! one, connection exists, two, it needs to be made
                if let Some(conn) = remote_connections.get_mut(&to){
                    conn.send(message).await;
                }else{
                    match chord.advert_of(to).await {
                        Some(data) => {
                            let addr = String::from_utf8_lossy(&data).to_string();
                            
                            // connect, add connection to remote connection list
                            let stream = TcpStream::connect(addr).await;
                            match stream{
                                Ok(stream) => {
                                    let tx = create_connection(stream, channel_in.clone());
                                    tx.send(Message::Introduction { id: self_id, as_peripheral: false }).await;
                                    tx.send(message).await;
                                    // Todo: needs redoing since node could respond with a different id
                                    // or multiple messages could be sent to the same node in quick succession
                                    remote_connections.insert(to, tx);
                                },
                                Err(e) => {
                                    println!("error: {}", e);
                                },
                            }
                             
                        },
                        None => {}, // Drop connection for now. Todo: Add some buffering rules
                    }
                }
            }
            
        }
        debug!("Exited main processing loop!");
    });
    (handle, channel_ret)
}

fn create_connection(mut stream: TcpStream, out: Sender<(SpiderId, SpiderMessage)>) -> Sender<Message>{
    let (tx, mut rx) = channel(50);
    let tx_ret = tx.clone();
    tokio::spawn(async move{
        let mut channel_id: Option<SpiderId> = None;
        let mut buffer = Vec::new();
        
        loop{
            let mut read_buffer = [0; 1024];
            select!{
                // if there is data from the stream, append it to the buffer
                // if the buffer can be deserialized, emit that message to out (until no more can be deserialized)
                read_len = stream.read(&mut read_buffer) => {
                    match read_len {
                        Ok(0) => { 
                            return; // No more data
                        }, 
                        Ok(len) => { // Append data to buffer
                            buffer.extend_from_slice(&read_buffer[..len]);
                        },
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                            continue; // try to read again
                        },
                        Err(e) =>{
                            println!("Encountered error reading from connection: {}", e);
                            // probably should terminate connection here, depending on error
                            break;
                        }
                    }
                    // attempt to deserialize buffer
                    let mut deserializer = Deserializer::from_slice(buffer.as_slice()).into_iter();

                    // if successful, truncate buffer, return deserialized struct
                    for result in &mut deserializer{
                        info!("Recieved message from client: {:?}", result);
                        match result{
                            Ok(msg) => {
                                match channel_id{
                                    // if there is an id, subsequent introductions should be droppped.
                                    Some(ref id) => {
                                        if let Message::Introduction{..} = msg{
                                            continue; // cant introduce more than once
                                        }
                                        out.send((id.clone(), SpiderMessage::Message(msg))).await;
                                    },
                                    // if there is no id, we must assign it. If incomming message is not an introduction, error the connection
                                    None => {
                                        if let Message::Introduction{id, as_peripheral} = msg{
                                            let new_id = if as_peripheral{
                                                SpiderId::Local(id)
                                            }else{
                                                SpiderId::Remote(id)
                                            };
                                            channel_id = Some(new_id.clone());
                                            let intro = SpiderMessage::Control(Control::Introduction{id: new_id.clone(), channel: tx.clone()});
                                            out.send((new_id.clone(), intro)).await;
                                        }else{
                                            // Todo return an error
                                            error!("Client sent message without introduction: {:?}", msg);
                                            return;
                                        }
                                    },
                                }
                                
                            },
                            Err(ref e) if e.classify() == Category::Eof => {
                                break; // if we have encountered an EOF, more information may arrive later
                            },
                            Err(e) => {
                                debug!("Encountered deserialization error: {}\n\tDeserialization buffer: {:?}", e,  String::from_utf8(buffer.clone()).unwrap());
                            },
                        }
                    }
                    buffer = buffer[deserializer.byte_offset()..].to_vec();
                },
                // if a message came through the channel, serialize it and send it on the stream
                msg = rx.recv() => {
                    let raw_data = serde_json::ser::to_string(&msg).expect("Failed to serialize struct");
                    stream.write(raw_data.as_bytes()).await;
                }
            }
        }
    });
    tx_ret
}

use std::io::ErrorKind;

use serde_json::{Deserializer, error::Category};
use tokio::{net::{ToSocketAddrs, TcpStream}, sync::mpsc::{channel, Sender, Receiver}, select, io::{AsyncReadExt, AsyncWriteExt}};
use tracing::{error, info};

use crate::message::{SpiderMessage, SpiderProtocol};


pub struct SpiderLink{
	self_id: u32,
	self_peripheral: bool,
	link_id: u32,
	link_peripheral: bool,
	out_tx: Sender<SpiderMessage>,
	in_rx: Receiver<SpiderMessage>,
}

impl SpiderLink{
	pub async fn connect<A: ToSocketAddrs>(addr: A, id: u32, as_peripheral: bool) -> Option<Self>{
		if let Ok(connection) = TcpStream::connect(addr).await {
			return SpiderLink::from_stream(connection, id, as_peripheral).await;
		}
		None
	}

	pub async fn from_stream(mut stream: TcpStream, id: u32, as_peripheral: bool) -> Option<Self>{
		let (out_tx, mut out_rx) = channel(50); 
		let (in_tx, in_rx) = channel(50);
		let link_id;
		let link_peripheral;

		// send introduction
		let intro = SpiderProtocol::Introduction { id, as_peripheral };
		let raw_data = serde_json::ser::to_string(&intro).expect("Failed to serialize struct");
        stream.write(raw_data.as_bytes()).await;
		
		let mut buffer = Vec::<u8>::new();
		// read introduction
		match read_stream(&mut stream, &mut buffer).await {
			Some(message) => {
				match message {
					SpiderProtocol::Introduction { id, as_peripheral } => {
						link_id = id;
						link_peripheral = as_peripheral;
					},
					_ => {
						return None;
					}
				}
			},
			None => {
				//Connection has closed
				return None;
			},
		}
		// relay messages
		tokio::spawn(async move{
			loop{
				select! {
					message = read_stream(&mut stream, &mut buffer) => {
						match message{
							Some(message) => {
								if let SpiderProtocol::Message(message) = message{
									in_tx.send(message).await;
								}
							},
							None => {
								//Connection has closed
								break;
							},
						}
					},
					msg = out_rx.recv() => {
						info!("In write, sending message: {:?}", msg);
                        let raw_data = serde_json::ser::to_string(&msg).expect("Failed to serialize struct");
                        stream.write(raw_data.as_bytes()).await;
					} 
				}
			}
		});
		Some(SpiderLink{
			self_id: id,
			self_peripheral: as_peripheral,
			link_id,
			link_peripheral,
			out_tx,
			in_rx,
		})
	}

	pub async fn send(&self, msg: SpiderMessage){
		self.out_tx.send(msg).await;
	}

	pub async fn recv(&mut self) -> Option<SpiderMessage>{
		self.in_rx.recv().await
	}

	pub fn self_id(&self) -> u32 {
		self.self_id
	}

	pub fn self_is_peripheral(&self) -> bool {
		self.self_peripheral
	}

	pub fn link_id(&self) -> u32 {
		self.link_id
	}

	pub fn link_is_peripheral(&self) -> bool {
		self.link_peripheral
	}

}




async fn read_stream(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> Option<SpiderProtocol>{
	loop{
		// Attempt to deserialize from the buffer
		let mut deserializer = Deserializer::from_slice(buffer.as_slice()).into_iter();
		
		match deserializer.next() {
			Some(result) => {
				match result{
					Ok(msg) => {
						*buffer = buffer[deserializer.byte_offset()..].to_vec();
						break Some(msg);
					},
					Err(ref e) if e.classify() == Category::Eof => {
						// if we have encountered an EOF, more information may arrive later
						// procede to read section
					},
					Err(e) => {
						error!("Encountered deserialization error: {}\n\tDeserialization buffer: {:?}", e, String::from_utf8(buffer.clone()).unwrap());
						break None;
					},
				}
			},
			None => {
				// if there is no next element, more may arrive after read
			},
		}
		
		
		// if there is insufficient data to deserialize, read some more
		let mut read_buffer = [0; 1024];
		match stream.read(&mut read_buffer).await {
			Ok(0) => {
				return None;
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
				return None;
			}
		}
	}
}
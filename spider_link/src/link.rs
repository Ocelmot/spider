
use std::{io::ErrorKind, future::Future};

use chacha20poly1305::{Key, Nonce, ChaCha20Poly1305, KeyInit, aead::{OsRng, Aead}};
use rand::RngCore;
use rsa::PublicKey;
use serde_json::{Deserializer, error::Category};
use tokio::{
	net::{
		ToSocketAddrs,
		TcpStream,
		TcpListener
	},
	sync::mpsc::{
		channel,
		Sender,
		Receiver
	},
	select,
	io::{AsyncReadExt, AsyncWriteExt}
};
use tracing::{error, info};

use crate::{message::{Frame, Message, Protocol}, SelfRelation, Relation};
#[derive(Debug)]
pub struct Link{
	own_relation: SelfRelation,
	other_relation: Relation,

	out_tx: Sender<Message>,
	in_rx: Option<Receiver<Message>>,
}

impl Link{
	pub async fn connect<A: ToSocketAddrs>(own_relation: SelfRelation, addr: A, relation: Relation) -> Option<Self>{
		if let Ok(connection) = TcpStream::connect(addr).await {
			let mut lb = LinkBuilder::from_stream(own_relation, connection);
			lb.set_other_relation(Some(relation));
			// println!("connect sending stream config");
			lb.send_stream_config().await;
			// println!("connect sent stream config");
			// println!("connect sending introduction");
			lb.send_introduction().await;
			// println!("connect sent introduction");
			// println!("connect reading stream config");
			lb.read_stream_config().await;
			// println!("connect read stream config");
			// println!("connect reading introduction");
			lb.read_introduction().await;
			// println!("connect read introduction");
			// process stream
			return Some(lb.process().await);
		}
		None
	}

	// Listener:
	pub fn listen<A: ToSocketAddrs + Send + 'static>(own_relation: SelfRelation, listen_addr: A) -> Receiver<Link>{
		let (tx, rx) = channel(50);
		// listen for connections,
		tokio::spawn(async move{
			let listener = TcpListener::bind(listen_addr).await.expect("failed to start Link listener");
			loop{
				let stream = if let Ok((stream, _)) = listener.accept().await{
					stream
				}else{
					return;
				};

				let local_tx = tx.clone();
				let local_own_relation = own_relation.clone();
				tokio::spawn(async move{
					let mut lb = LinkBuilder::from_stream(local_own_relation, stream);
					// println!("listen reading stream config");
					lb.read_stream_config().await;
					// println!("listen read stream config");
					// println!("listen reading introduction");
					lb.read_introduction().await;
					// println!("listen read introduction");
					// println!("listen sending stream config");
					lb.send_stream_config().await;
					// println!("listen sent stream config");
					// println!("listen sending introduction");
					lb.send_introduction().await;
					// println!("listen sent introduction");
					// process stream,
					let link = lb.process().await;
					// emit Link on channel,
					local_tx.send(link).await;
				});
			}
			
		});
		rx
	}

	pub fn own_relation(&self) -> &SelfRelation{
		&self.own_relation
	}

	pub fn other_relation(&self) -> &Relation{
		&self.other_relation
	}

	pub async fn send(&self, msg: Message){
		self.out_tx.send(msg).await;
	}

	pub async fn recv(&mut self) -> Option<Message>{
		match &mut self.in_rx{
			Some(in_rx) => in_rx.recv().await,
			None => None,
		}	
	}

	pub fn take_recv(&mut self) -> Option<Receiver<Message>>{
		self.in_rx.take()
	}
}





struct LinkBuilder{
	stream: TcpStream,
	buffer: Vec<u8>,

	own_relation: SelfRelation,
	own_key: [u8; 32],
	own_nonce: [u8; 12],

	other_relation: Option<Relation>,
	other_key: Option<[u8; 32]>,
	other_nonce: Option<[u8; 12]>,
}


impl LinkBuilder{
	pub fn from_stream(own_relation: SelfRelation, stream: TcpStream) -> Self{

		let own_key = ChaCha20Poly1305::generate_key(&mut OsRng).into();
		let mut own_nonce = [0u8; 12];
		OsRng.fill_bytes(&mut own_nonce);

		Self{
			stream,
			buffer: Vec::new(),

			own_relation,
			own_key,
			own_nonce,

			other_relation: None,
			other_key: None,
			other_nonce: None,
		}
	}

	pub fn set_other_relation(&mut self, relation: Option<Relation>){
		self.other_relation = relation;
	}

	async fn send_stream_config(&mut self){
		//println!("sending stream config");
		let mut raw_data = Vec::new();
		raw_data.extend_from_slice(&self.own_key);
		raw_data.extend_from_slice(&self.own_nonce);

		let other_relation = self.other_relation.as_ref().expect("stream config can only be sent after setting other_relation");
		let data = {
			// encrypt with other party's key
			let key = other_relation.id.as_pub_key().expect("Failed to convert to key");
			let padding = rsa::PaddingScheme::new_pkcs1v15_encrypt();
			let mut rng = rand::thread_rng();
			let enc_data = key.encrypt(&mut rng, padding, &raw_data).expect("Failed to encrypt");
			// build and send frame
			enc_data
		};
		self.write_frame(data).await;
		// println!("sent stream config");
	}

	async fn read_stream_config(&mut self){
		//println!("reading stream config");
		let enc_data = if let Some(enc_data) = self.read_frame().await {
			enc_data
		} else {
			self.stream.shutdown().await;
			return;
		};
		

		let dec_data = { // ensure that encryption items are not held across await
			// decrypt data with our key
			let padding = rsa::PaddingScheme::new_pkcs1v15_encrypt();
			self.own_relation.private_key().decrypt(padding, &enc_data).expect("Failed to decrypt")
		};
		let stream_key: [u8; 32] = dec_data[0..32].try_into().expect("wrong length");
		let stream_nonce: [u8; 12] = dec_data[32..(32+12)].try_into().expect("wrong length");
		self.other_key = Some(stream_key);
		self.other_nonce = Some(stream_nonce);
		// println!("saved stream config");
	}


	async fn send_introduction(&mut self){
		//println!("sending introduction");
		// serialize introduction message (stream encryption)
		let intro = Protocol::Introduction {
			id: self.own_relation.relation.id.clone(),
			role: self.own_relation.relation.role,
		};
		let raw_data = serde_json::ser::to_vec(&intro).expect("Failed to serialize struct");

		let data = self.own_encrypt(&raw_data);

		self.write_frame(data).await;
	}

	async fn read_introduction(&mut self){
		//println!("reading introduction");
		// read packet data
		let enc_data = if let Some(enc_data) = self.read_frame().await {
			enc_data
		} else {
			eprintln!("Failed to read frame!");
			self.stream.shutdown().await;
			return;
		};

		let dec_data = self.other_decrypt(&enc_data);

		// deserialize into introduction message
		let prot: Protocol = serde_json::de::from_slice(&dec_data).expect("Failed to deserialize introduction");
		// create Relation
		if let Protocol::Introduction { id, role} = prot {
			let other_rel = Relation{
				id,
				role,
			};
			match &self.other_relation {
				Some(existing_other) => {
					println!("other relation exists: {:?}", other_rel);
					if *existing_other != other_rel {
						println!("other relation differs from current self");
						// error has occured, this is not who we expected to connect to, close
						self.stream.shutdown().await;
						return;
					}else{
						// println!("other relation equals current self");
					}
				},
				None => {
					self.other_relation = Some(other_rel);
					// println!("Other relation recieved!")
				},
			}
		}else{
			self.stream.shutdown().await;
			return;
		}
	}

	async fn read_frame(&mut self) -> Option<Vec<u8>> {
		loop{
			// Attempt to deserialize from the buffer
			let mut deserializer = Deserializer::from_slice(self.buffer.as_slice()).into_iter();
			
			match deserializer.next() {
				Some(result) => {
					match result{
						Ok(msg) => {
							let msg: Frame = msg;
							self.buffer = self.buffer[deserializer.byte_offset()..].to_vec();
							break Some(msg.data);
						},
						Err(ref e) if e.classify() == Category::Eof => {
							// if we have encountered an EOF, more information may arrive later
							// procede to read section
						},
						Err(e) => {
							error!("Encountered deserialization error: {}\n\tDeserialization buffer: {:?}", e, String::from_utf8(self.buffer.clone()).unwrap());
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
			match self.stream.read(&mut read_buffer).await {
				Ok(0) => {
					return None;
				}, 
				Ok(len) => { // Append data to buffer
					self.buffer.extend_from_slice(&read_buffer[..len]);
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

	async fn write_frame(&mut self, data: Vec<u8>){
		let frame = Frame{data};
		let bytes = serde_json::ser::to_vec(&frame).expect("failed to serialize frame");
		self.stream.write_all(&bytes).await;
	}

	fn own_encrypt(&self, data: &[u8]) -> Vec<u8> {
		let own_key = Key::from(self.own_key);
		let own_nonce = Nonce::from(self.own_nonce);
		
		let cypher = ChaCha20Poly1305::new(&own_key);

		cypher.encrypt(&own_nonce, data).unwrap()
	}

	fn other_decrypt(&self, data: &[u8]) -> Vec<u8>{
		let other_key = Key::from(self.other_key.unwrap());
		let other_nonce = Nonce::from(self.other_nonce.unwrap());

		let cypher = ChaCha20Poly1305::new(&other_key);

		cypher.decrypt(&other_nonce, data).unwrap()
	}

	pub async fn process(mut self) -> Link{
		let own_relation = self.own_relation.clone();
		let other_relation = self.other_relation.clone().unwrap();

		let (out_tx, mut out_rx) = channel(50); 
		let (in_tx, in_rx) = channel(50);

		tokio::spawn(async move{
			loop{
				select! {
					data = self.read_frame() => {
						match data{
							Some(data) => {
								// must also decrypt here
								let decrypted_data = self.other_decrypt(&data);
								// deserialize frame data
								let proto: Protocol = serde_json::from_slice(&decrypted_data).unwrap();
								match proto {
									Protocol::Introduction { id, role} => {
										panic!("it is an error to send a second introduction");
									},
									Protocol::Message(msg) => {
										in_tx.send(msg).await;
									},
								}
							},
							None => {
								//Connection has closed
								break;
							},
						}
					},
					msg = out_rx.recv() => {
						let msg = if let Some(msg) = msg{
							msg
						} else {
							break; // connection closed, shutdown
						};
						let protocol = Protocol::Message(msg);
						let raw_data = serde_json::ser::to_vec(&protocol).expect("Failed to serialize struct");
						// encrypt using stream cypher here
						let encrypted_data = self.own_encrypt(&raw_data);
						self.write_frame(encrypted_data).await;
					} 
				}
			}
		});



		Link{
			own_relation,
			other_relation,
			out_tx,
			in_rx: Some(in_rx),
		}
	}
}




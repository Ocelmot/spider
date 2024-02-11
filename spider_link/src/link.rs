
//! The Link module manages the actual creation of the basic connection
//! between any two nodes on the spider network.
//! 
//! The Link also provides encryption and serialization to the
//! [Messages](Message) that are sent through it.


use std::{io::ErrorKind, sync::Arc};

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
	sync::{mpsc::{
		channel,
		Sender,
		Receiver, error::SendError
	}, Mutex, Notify},
	select,
	io::{AsyncReadExt, AsyncWriteExt}, task::JoinHandle
};
use tracing::{error, info};

use crate::{message::{Frame, Message, Protocol, KeyRequest}, SelfRelation, Relation};

/// A Link is the connection between two nodes of the network.
/// It sends and recieves [Messages](Message), and is encrypted.
#[derive(Debug)]
pub struct Link{
	self_relation: SelfRelation,
	other_relation: Relation,

	out_tx: Sender<Message>,
	in_rx: Option<Receiver<Message>>,

	notify_exit: Arc<Notify>,
	handle: JoinHandle<()>,
}

impl Link{
	/// Establish a connection between two nodes. This requires the
	/// SelfRelation of the local node, and the IP Address and
	/// relation of the remote node.
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
			lb.read_stream_config(&None).await;
			// println!("connect read stream config");
			// println!("connect reading introduction");
			lb.read_introduction().await;
			// println!("connect read introduction");
			// process stream
			return Some(lb.process().await);
		}
		None
	}

	/// Listen for incoming Links with a SelfRelation and a bind address.
	/// Returns both a channel through which new Links will be sent, and a
	/// Mutex to control if this listener will respond to queries of its
	/// private key.
	pub fn listen<A: ToSocketAddrs + Send + 'static>(own_relation: SelfRelation, listen_addr: A) -> (Receiver<Link>, Arc<Mutex<Option<String>>>)
		{
		let (tx, rx) = channel(50);
		let kr: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
		let kr_ret = kr.clone();
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
				let local_kr = kr.clone();
				tokio::spawn(async move{
					let mut lb = LinkBuilder::from_stream(local_own_relation, stream);
					// println!("listen reading stream config");
					let done = lb.read_stream_config(&&local_kr.lock().await).await;
					if done {
						return;
					}
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
		(rx, kr_ret)
	}

	/// Request the private key of a link listener at an IP address.
	pub async fn key_request<A: ToSocketAddrs + Send + 'static>(addr: A) -> Option<KeyRequest>{
		let mut sock = match TcpStream::connect(addr).await {
			Ok(sock) => sock,
			Err(_) => return None,
		};
		let frame = Frame{ data: b"KEY_REQUEST".to_vec() };
		let send_buf = serde_json::to_vec(&frame).expect("frame should serialize");
		sock.write(&send_buf).await;
		let mut buf = String::new();
		sock.read_to_string(&mut buf).await;
		let frame = serde_json::de::from_str::<Frame>(&buf).ok()?;
		let key = serde_json::de::from_slice::<KeyRequest>(&frame.data);
		key.ok()
	}

	/// Returns the SelfRelation of this Link
	pub fn self_relation(&self) -> &SelfRelation{
		&self.self_relation
	}

	/// Returns the remote Relation of this Link
	pub fn other_relation(&self) -> &Relation{
		&self.other_relation
	}

	/// Sends a Message through the link
	pub async fn send(&self, msg: Message) -> Result<(), SendError<Message>>{
		self.out_tx.send(msg).await
	}

	/// Recieves a Message from the Link, if the reciever has not been taken
	pub async fn recv(&mut self) -> Option<Message>{
		match &mut self.in_rx{
			Some(in_rx) => in_rx.recv().await,
			None => None,
		}	
	}

	/// Take the recieving channel from this link if it has not already been
	/// taken. This can be used to process sending and recieving on
	/// different threads or tasks.
	pub fn take_recv(&mut self) -> Option<Receiver<Message>>{
		self.in_rx.take()
	}

	/// Terminates the Link in both directions
	pub async fn terminate(self){
		self.notify_exit.notify_waiters();
		self.handle.await;
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

	async fn read_stream_config(&mut self, enable_key_request: &Option<String>) -> bool{
		//println!("reading stream config");
		let enc_data = if let Some(enc_data) = self.read_frame().await {
			enc_data
		} else {
			self.stream.shutdown().await;
			return true;
		};
		
		// if key is requested and enabled, respond with that instead.
		// Then, quit.
		if enc_data == b"KEY_REQUEST"{
			match enable_key_request {
				Some(name) => {
					self.respond_key_request(name.clone()).await;
				},
				None => {},
			}
			self.stream.shutdown().await;
			return true;
		}

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
		return false;
	}

	async fn respond_key_request(&mut self, name: String) {
		let request = KeyRequest{
			key: self.own_relation.relation.id.clone(),
			name
		};
		let data = serde_json::ser::to_vec(&request).expect("request should serialize");
		self.write_frame(data).await;
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

		let notify_exit = Arc::new(Notify::new());
		let notify_exit_copy = notify_exit.clone();

		let handle = tokio::spawn(async move{
			loop{
				select! {
					_ = notify_exit.notified() => {
						break; // exit the loop to stop the processor
					}
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
			self_relation: own_relation,
			other_relation,
			out_tx,
			in_rx: Some(in_rx),
			notify_exit: notify_exit_copy,
			handle,
		}
	}
}




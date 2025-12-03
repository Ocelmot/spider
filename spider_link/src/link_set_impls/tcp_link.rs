use std::{
    collections::VecDeque,
    io::{Read, Write},
    mem,
    sync::Arc,
};

use crate::{
    LinkError, LinkResult, Relation, SelfRelation, error::{ErrorKind, Problem, ProblemWrap}, identified_link::IdentifiedLink, link_set::links::Link, link_set_impls::{LinkImplError, LinkImplResult}, message::KeyRequest
};

use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, Key, KeyInit, Nonce};
use link_set::{links::LinkReader, LinkProtocol};
use num_bigint::BigUint;
use rand::{rngs::OsRng, RngCore};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream, ToSocketAddrs,
    },
    sync::{
        mpsc::{channel, Receiver},
        Mutex,
    },
};
use tracing::{debug, error};

/// Manages a TCP connection to another member of the spider network.
///  
/// Encrypts the data sent across it.
pub struct TCPLink {
    self_relation: SelfRelation,
    self_key: [u8; 32],
    self_nonce: BigUint,

    other_relation: Relation,
    other_key: [u8; 32],
    other_nonce: BigUint,

    socket_reader: Option<OwnedReadHalf>,
    socket_writer: OwnedWriteHalf,
    read_len: Option<u32>,
    read_buffer: VecDeque<u8>,
    is_closed: bool,
}

impl TCPLink {
    /// Listen for new incoming TCPLinks
    ///
    /// This listener will not respond to requests for its public key
    pub fn listen<A: ToSocketAddrs + Send + 'static>(
        self_relation: SelfRelation,
        listen_addr: A,
    ) -> Receiver<Self> {
        let key_req = Arc::new(Mutex::new(None));
        Self::listen_key_req(self_relation, listen_addr, key_req)
    }

    /// Sets up a listener for new incoming TCPLinks
    ///
    /// The listener is also passed a key_req which is an
    /// Arc<Mutex<Option<String>>>. This can be used to enable or disable
    /// responses to the key request function. If enabled, it also indicates
    /// what the human readable name of this device is.
    pub fn listen_key_req<A: ToSocketAddrs + Send + 'static>(
        self_relation: SelfRelation,
        listen_addr: A,
        key_req: Arc<Mutex<Option<String>>>,
    ) -> Receiver<Self> {
        let (tx, rx) = channel(50);

        // listen for connections,
        tokio::spawn(async move {
            let listener = TcpListener::bind(listen_addr)
                .await
                .expect("failed to start TCPLink listener");
            loop {
                let socket = if let Ok((stream, _)) = listener.accept().await {
                    stream
                } else {
                    return;
                };
                let (mut socket_reader, mut socket_writer) = socket.into_split();

                let local_tx = tx.clone();
                let local_self_relation = self_relation.clone();
                let local_key_req = key_req.clone();
                tokio::spawn(async move {
                    // Recv other's stream details
                    let Some((other_key, mut other_nonce)) =
                        recv_stream_init(&mut socket_reader, &local_self_relation).await?
                    else {
                        if let Some(ref name) = *local_key_req.lock().await {
                            let _ =
                                respond_key_request(&mut socket_writer, &local_self_relation, name)
                                    .await;
                            let _ = socket_reader
                                .reunite(socket_writer)
                                .unwrap()
                                .shutdown()
                                .await;
                        }
                        return LinkResult::Ok(());
                    };

                    // Recv other's introduction
                    let mut read_len = None;
                    let mut read_buffer = VecDeque::new();
                    let other_relation = recv_introduction(
                        &mut socket_reader,
                        other_key,
                        &mut other_nonce,
                        &mut read_len,
                        &mut read_buffer,
                    )
                    .await?;

                    // Send our stream details
                    let (self_key, mut self_nonce) =
                        send_stream_init(&mut socket_writer, &other_relation).await?;

                    // Send our introduction
                    send_introduction(
                        &mut socket_writer,
                        self_key,
                        &mut self_nonce,
                        &local_self_relation,
                    )
                    .await?;

                    let tcp_link = Self {
                        self_relation: local_self_relation,
                        self_key,
                        self_nonce,

                        other_relation,
                        other_key,
                        other_nonce,

                        socket_reader: Some(socket_reader),
                        socket_writer,
                        read_len,
                        read_buffer,
                        is_closed: false,
                    };

                    // emit Link on channel,
                    local_tx.send(tcp_link).await.wrap()?;
                    LinkResult::Ok(())
                });
            }
        });
        rx
    }

    /// Request the private key of a link listener at an IP address.
    pub async fn key_request<A: ToSocketAddrs + Send + 'static>(addr: A) -> Option<KeyRequest> {
        let mut sock = TcpStream::connect(addr).await.ok()?;

        let data = b"KEY_REQUEST";
        sock.write_u32(data.len() as u32).await.ok()?;
        sock.write_all(data).await.ok()?;

        let len = sock.read_u32().await.ok()?;
        let mut buf = String::with_capacity(len as usize);
        let read_count = sock.take(len as u64).read_to_string(&mut buf).await.ok()?;

        if read_count < len as usize {
            // could not read enough bytes
            return None;
        }

        serde_json::de::from_str::<KeyRequest>(&buf).ok()
    }

    /// Establish a connection between two nodes. This requires the
    /// SelfRelation of the local node, and the IP Address and
    /// relation of the remote node.
    pub async fn connect<A: ToSocketAddrs>(
        self_relation: SelfRelation,
        other_relation: Relation,
        addr: A,
    ) -> LinkResult<Self> {
        // Connect socket
        let socket = TcpStream::connect(addr).await.wrap()?;
        let (mut socket_reader, mut socket_writer) = socket.into_split();

        // Send stream encryption init
        let (self_key, mut self_nonce) =
            send_stream_init(&mut socket_writer, &other_relation).await?;

        // Send our relation and id
        send_introduction(
            &mut socket_writer,
            self_key,
            &mut self_nonce,
            &self_relation,
        )
        .await?;

        // Receive other's stream encryption init
        let Some((other_key, mut other_nonce)) =
            recv_stream_init(&mut socket_reader, &self_relation).await?
        else {
            return Err(LinkError::new().msg("Received key request on connection"));
        };

        // Receive other's relation and id.
        let mut read_len = None;
        let mut read_buffer = VecDeque::new();
        let recvd_rel = recv_introduction(
            &mut socket_reader,
            other_key,
            &mut other_nonce,
            &mut read_len,
            &mut read_buffer,
        )
        .await?;

        // Check that other is who we expected
        if recvd_rel != other_relation {
            return Err(LinkError::new()
                .msg("Received relation is not equal to intended recipient relation."));
        }

        // Return the connected TCPLink
        Ok(TCPLink {
            self_relation,
            self_key,
            self_nonce,

            other_relation,
            other_key,
            other_nonce,

            socket_reader: Some(socket_reader),
            socket_writer,
            read_len,
            read_buffer,

            is_closed: false,
        })
    }

    async fn read_chunk(&mut self) -> LinkResult<Vec<u8>> {
        recv_chunk(
            self.socket_reader
                .as_mut()
                .wrap_msg("Reader has been taken")?,
            self.other_key,
            &mut self.other_nonce,
            &mut self.read_len,
            &mut self.read_buffer,
        )
        .await
    }

    async fn write_chunk(&mut self, data: &Vec<u8>) -> LinkResult {
        send_chunk(
            &mut self.socket_writer,
            self.self_key,
            &mut self.self_nonce,
            data,
        )
        .await
    }


}

impl IdentifiedLink for TCPLink {
    // fn self_relation(&self) -> &SelfRelation {
    //     &self.self_relation
    // }

    fn other_relation(&self) -> &Relation {
        &self.other_relation
    }
}

async fn send_stream_init(
    writer: &mut OwnedWriteHalf,
    other_relation: &Relation,
) -> LinkResult<([u8; 32], BigUint)> {
    // Generate stream keys
    let key: [u8; 32] = ChaCha20Poly1305::generate_key(&mut OsRng).into();
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);

    let mut raw_data = Vec::new();
    raw_data.extend_from_slice(&key);
    raw_data.extend_from_slice(&nonce);

    let data = other_relation.encrypt(&raw_data);
    writer
        .write_u32(data.len() as u32)
        .await
        .wrap_msg("Failed to write stream init length")?;
    writer
        .write_all(&data)
        .await
        .wrap_msg("Failed to write stream init data")?;

    let nonce = BigUint::from_bytes_be(&nonce);
    Ok((key, nonce))
}

async fn recv_stream_init(
    reader: &mut OwnedReadHalf,
    self_relation: &SelfRelation,
) -> LinkResult<Option<([u8; 32], BigUint)>> {
    let len = reader.read_u32().await.wrap()?;
    let mut buf = Vec::with_capacity(len as usize);
    reader.take(len as u64).read_to_end(&mut buf).await.wrap()?;

    if buf == b"KEY_REQUEST" {
        return Ok(None);
    }

    let decrypted = self_relation
        .decrypt(&buf)
        .ok_or(LinkError::new())
        .msg("Failed to decrypt stream init")?;
    let key: [u8; 32] = decrypted[..32]
        .try_into()
        .wrap_msg("Failed to deserialize key")?;
    let nonce_bytes = &decrypted[32..];

    let nonce = BigUint::from_bytes_be(nonce_bytes);

    Ok(Some((key, nonce)))
}

async fn respond_key_request(
    writer: &mut OwnedWriteHalf,
    self_relation: &SelfRelation,
    name: &String,
) -> LinkResult {
    let request = KeyRequest {
        key: self_relation.relation.id.clone(),
        name: name.clone(),
    };
    let data = serde_json::ser::to_vec(&request).expect("request should serialize");

    writer.write_u32(data.len() as u32).await.wrap()?;
    writer.write_all(&data).await.wrap()?;
    Ok(())
}

/// Send our id and role to the other side.
///
/// This is encrypted with the stream encryption.
async fn send_introduction(
    writer: &mut OwnedWriteHalf,
    key: [u8; 32],
    nonce: &mut BigUint,
    self_relation: &SelfRelation,
) -> LinkResult {
    let data = self_relation.relation.to_base64();

    send_chunk(writer, key, nonce, data.as_bytes()).await
}

async fn recv_introduction(
    reader: &mut OwnedReadHalf,
    key: [u8; 32],
    nonce: &mut BigUint,
    length: &mut Option<u32>,
    buffer: &mut VecDeque<u8>,
) -> LinkResult<Relation> {
    let data = recv_chunk(reader, key, nonce, length, buffer).await?;
    let rel = Relation::from_base64(&String::from_utf8(data).wrap()?)
        .ok_or(LinkError::new().msg("Failed to convert relation from base64"))?;
    Ok(rel)
}

async fn send_chunk(
    writer: &mut OwnedWriteHalf,
    key: [u8; 32],
    nonce: &mut BigUint,
    data: &[u8],
) -> LinkResult {
    let own_key = Key::from(key);
    let mut nonce_vec = nonce.to_bytes_be();
    nonce_vec.resize(12, 0);
    let nonce_bytes = TryInto::<[u8; 12]>::try_into(nonce_vec).unwrap();
    let nonce_bytes = Nonce::from(nonce_bytes);

    let cipher = ChaCha20Poly1305::new(&own_key);

    let msg = cipher.encrypt(&nonce_bytes, data).unwrap();
    debug!("chunk write with len {}", msg.len());
    writer.write_u32(msg.len() as u32).await.wrap()?;
    writer.write_all(&msg).await.wrap()?;
    *nonce += 1u32;
    Ok(())
}

// #[instrument(ret)]
async fn recv_chunk(
    reader: &mut OwnedReadHalf,
    key: [u8; 32],
    nonce: &mut BigUint,
    length: &mut Option<u32>,
    buffer: &mut VecDeque<u8>,
) -> LinkResult<Vec<u8>> {
    let mut buf = [0u8; 1024];

    let len = match length {
        Some(len) => *len,
        None => {
            loop {
                // trace!("starting chunk read...");
                if buffer.len() >= 4 {
                    let mut len_bytes = [0u8; 4];
                    buffer.read_exact(&mut len_bytes).wrap()?;
                    let len = u32::from_be_bytes(len_bytes);
                    // debug!("chunk read with len {}", len);
                    length.replace(len);
                    break len;
                }

                let x = reader.read(&mut buf).await.wrap()?;
                if x == 0 {
                    return Err(LinkError::new().problem(ErrorKind::Closed));
                }
                buffer.write(&buf[..x]).wrap()?;
            }
        }
    };

    loop {
        // if we have enough data in the read buffer, return it
        if len <= buffer.len() as u32 {
            let mut ciphertext = Vec::with_capacity(len as usize);
            buffer
                .take(len.into())
                .read_to_end(&mut ciphertext)
                .wrap()?;

            // Decrypt buffer
            let their_key = Key::from(key);
            let nonce_bytes = TryInto::<[u8; 12]>::try_into(nonce.to_bytes_be()).unwrap();
            let nonce_bytes = Nonce::from(nonce_bytes);

            let cipher = ChaCha20Poly1305::new(&their_key);
            let plaintext = cipher.decrypt(&nonce_bytes, ciphertext.as_slice());
            if plaintext.is_err() {
                error!("Decrypting buffer returned {:?}", plaintext);
            }

            // let ret = ret?;
            *nonce += 1u32;
            *length = None;

            return Ok(plaintext.unwrap());
        }

        // otherwise, read some more
        let x = reader.read(&mut buf).await.wrap()?;
        buffer.write(&buf[..x]).wrap()?;
    }
}

pub struct TcpLinkReader(Receiver<LinkProtocol>);

impl LinkReader for TcpLinkReader {
    async fn read(&mut self) -> Result<LinkProtocol, LinkImplError> {
        self.0.recv().await.ok_or(LinkImplError::Closed)
    }
}

impl Link for TCPLink {

    async fn send(&mut self, msg: LinkProtocol) -> Result<(), impl std::error::Error + Send + Sync + 'static> {
        debug!("Sending msg {:?}", msg);
        let bytes = msg.serialize();
        self.write_chunk(&bytes)
            .await
            .map_err(|_| LinkImplError::Closed)
    }

    async fn recv(&mut self) -> Result<LinkProtocol, impl std::error::Error + Send + Sync + 'static> {
        let data = self.read_chunk().await.map_err(|_| LinkImplError::Closed)?;
        let mut data = VecDeque::from(data);
        let ret = LinkProtocol::deserialize(&mut data).map_err(|_| LinkImplError::Deserialize);
        debug!("Receiving msg: {:?}", ret);
        ret
    }

    /// Splits the read portion and write portion of this TCP Link, returning a
    /// Receiver<LinkProtocol> from which incoming messages can be received.
    fn take_reader(&mut self) -> Result<impl LinkReader + 'static, LinkImplError> {
        let mut reader = self
            .socket_reader
            .take()
            .ok_or(LinkImplError::ReceiverTaken)?;
        let other_key = self.other_key.clone();
        let mut other_nonce = self.other_nonce.clone();
        let mut read_len = self.read_len.take();
        let mut read_buffer = mem::take(&mut self.read_buffer);
        let (tx, rx) = channel(10);
        tokio::task::spawn(async move {
            loop {
                let chunk = recv_chunk(
                    &mut reader,
                    other_key,
                    &mut other_nonce,
                    &mut read_len,
                    &mut read_buffer,
                )
                .await;
                let chunk = chunk.map_err(|_| LinkImplError::Closed)?;
                let mut chunk = VecDeque::from(chunk);
                let x =
                    LinkProtocol::deserialize(&mut chunk).map_err(|_| LinkImplError::Deserialize);
                debug!("Receiving msg (taken reader): {:?}", x);
                tx.send(x?).await.map_err(|_| LinkImplError::Closed)?;
            }
            #[allow(unreachable_code)]
            Ok::<(), LinkImplError>(())
        });

        Ok(TcpLinkReader(rx))
    }

    fn max_size(&self) -> u32 {
        let mut limit = u32::MAX;
        limit -= 4; // for the chunk length indicator
        limit -= 16; // for encryption tag size
        limit -= 1024; // just in case
        limit
    }

    fn is_closed(&mut self) -> bool {
        self.is_closed
    }
}

#[cfg(test)]
mod tests {

    use tokio::join;

    use super::*;

    async fn socket_pair(port: u16) -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();

        join!(
            async { TcpStream::connect(("127.0.0.1", port)).await.unwrap() },
            async {
                let (socket, _) = listener.accept().await.unwrap();
                socket
            }
        )
    }

    #[tokio::test]
    async fn send_stream_init_test() {
        let (s, t) = socket_pair(1930).await;
        let recv_rel = SelfRelation::debug_get(0);
        let (_sr, mut sw) = s.into_split();
        let (sent_key, sent_nonce) = send_stream_init(&mut sw, &recv_rel.relation).await.unwrap();

        let (mut tr, _tw) = t.into_split();
        let Some((recvd_key, recvd_nonce)) = recv_stream_init(&mut tr, &recv_rel).await.unwrap()
        else {
            panic!("Did not recv key and nonce");
        };
        assert_eq!(sent_key, recvd_key);
        assert_eq!(sent_nonce, recvd_nonce);
    }

    #[tokio::test]
    async fn send_introduction_test() {
        let (s, t) = socket_pair(1931).await;
        let send_rel = SelfRelation::debug_get(0);
        let (_sr, mut sw) = s.into_split();

        // Generate stream keys/nonce since we didn't call stream init
        let key: [u8; 32] = ChaCha20Poly1305::generate_key(&mut OsRng).into();
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let nonce = BigUint::from_bytes_be(&nonce);

        // send introduction
        let send_key = key.clone();
        let mut send_nonce = nonce.clone();
        send_introduction(&mut sw, send_key, &mut send_nonce, &send_rel)
            .await
            .unwrap();

        // recv introduction
        let (mut tr, _tw) = t.into_split();
        let recv_key = key.clone();
        let mut recv_nonce = nonce.clone();
        let mut read_len = None;
        let mut read_buffer = VecDeque::new();
        let recvd_rel = recv_introduction(
            &mut tr,
            recv_key,
            &mut recv_nonce,
            &mut read_len,
            &mut read_buffer,
        )
        .await
        .unwrap();

        assert_eq!(send_rel.relation, recvd_rel);
        assert_eq!(send_nonce, recv_nonce);
    }

    #[tokio::test]
    async fn listen_connect_test() {
        let listen_rel = SelfRelation::debug_get(0);

        let mut listener = TCPLink::listen(listen_rel.clone(), "127.0.0.1:1940");

        let connect_rel = SelfRelation::debug_get(1);
        let mut connect_link =
            TCPLink::connect(connect_rel.clone(), listen_rel.relation, "127.0.0.1:1940")
                .await
                .unwrap();

        let mut listen_link = listener.recv().await.unwrap();

        let data = b"test_data".to_vec();
        let msg = LinkProtocol::MsgSlice {
            epoch: 2,
            seq: 55,
            seq_len: data.len() as u64,
            first_index: 0,
            data,
        };
        connect_link.send(msg.clone()).await.unwrap();

        let recvd_msg = listen_link.recv().await.unwrap();

        assert_eq!(msg, recvd_msg);
    }

    #[tokio::test]
    async fn key_request_pass() {
        let listen_rel = SelfRelation::debug_get(0);

        let name = String::from("TEST NAME!");
        let key_req = Arc::new(Mutex::new(Some(name.clone())));
        let _listener = TCPLink::listen_key_req(listen_rel.clone(), "127.0.0.1:1941", key_req);

        let recvd_key_request = TCPLink::key_request("127.0.0.1:1941").await.unwrap();

        assert_eq!(name, recvd_key_request.name);
        assert_eq!(listen_rel.relation.id, recvd_key_request.key);
    }

    #[tokio::test]
    async fn key_request_fail() {
        let listen_rel = SelfRelation::debug_get(0);

        let key_req = Arc::new(Mutex::new(None));
        let _listener = TCPLink::listen_key_req(listen_rel.clone(), "127.0.0.1:1942", key_req);

        let recvd_key_request = TCPLink::key_request("127.0.0.1:1942").await;

        assert!(recvd_key_request.is_none());
    }
}

//! The transport module for TCP

use std::sync::Arc;

use link_set::{
    adaptors::peekable::Peekable,
    link_impls::TcpLink,
    links::{Link, LinkConnector},
};
use tokio::{
    net::ToSocketAddrs,
    sync::{mpsc::Sender, Mutex},
    task::JoinHandle,
};

use crate::{
    error::{ErrorKind, ProblemWrap},
    link_impls::authenticated::{connect_link_encrypting, Authenticated},
    message::KeyRequest,
    transports::LinkListener,
    LinkResult, Relation, SelfRelation,
};


/// The string tag to indicate the Iroh connector scheme
pub const TCP_SCHEME: &'static str = "auth_tcp";

const KEY_REQ_IDENTIFIER: &'static [u8; 8] = &b"SPDRKYRQ";

/// Implementation of [link_set::links::LinkConnector] that wraps TCP with
/// encryption
pub struct TcpConnector {
    sr: SelfRelation,
    rel: Relation,
}

impl TcpConnector {
    /// Create a new connector. Requires the [SelfRelation] and [Relation] to
    /// ensure the handshake is with the correct identity
    pub fn new(sr: SelfRelation, rel: Relation) -> Self {
        Self { sr, rel }
    }
}

impl LinkConnector for TcpConnector {
    fn scheme(&self) -> &'static str {
        "auth_tcp"
    }

    async fn connect(
        &self,
        addr: String,
    ) -> Result<impl Link + 'static, impl std::error::Error + Send + Sync + 'static> {
        let tcp_link = TcpLink::connect(addr).await.wrap()?;
        let (_rel, encrypted) =
            connect_link_encrypting(self.sr.clone(), self.rel.clone(), tcp_link).await?;

        LinkResult::Ok(encrypted)
    }
}

/// Request the base at addr for its key. It may have its responses disabled, in
/// which case this will error.
pub async fn key_request<A: ToSocketAddrs + Send + 'static>(addr: A) -> LinkResult<KeyRequest> {
    let mut link = TcpLink::connect(addr).await.wrap()?;
    link.send(KEY_REQ_IDENTIFIER.to_vec()).await.wrap()?;

    let request = link.recv().await.wrap()?;
    KeyRequest::from_bytes(&request)
}

/// Listener for encryption wrapped tcp links
pub struct TcpListener {
    listen_addr: String,
    key_req: Arc<Mutex<Option<String>>>,
}

impl TcpListener {
    /// Create a new listener for tcp links wrapped with encryption
    pub fn new(listen_addr: String, key_req: Arc<Mutex<Option<String>>>) -> Self {
        Self {
            listen_addr,
            key_req,
        }
    }
}

impl LinkListener for TcpListener {
    fn scheme(&self) -> &'static str {
        "auth_tcp"
    }

    fn listen(
        &self,
        sr: SelfRelation,
        sender: Sender<Authenticated>,
    ) -> JoinHandle<LinkResult<()>> {
        let listen_addr = self.listen_addr.clone();
        let key_req = self.key_req.clone();
        tokio::spawn(async move {
            let mut tcp_listener = TcpLink::listen(&listen_addr)
                .await
                .wrap_msg("Tcp Listen Failed")?;
            while let Some(tcp_link) = tcp_listener.recv().await {
                let mut tcp_link = Peekable::new(tcp_link);
                let spawn_sender = sender.clone();
                let spawn_sr = sr.clone();
                let local_key_req = key_req.clone();
                tokio::spawn(async move {
                    let peeked = tcp_link.peek().await.wrap_msg("peek failed")?;
                    if peeked.starts_with(KEY_REQ_IDENTIFIER) {
                        // handle key req
                        if let Some(ref name) = *local_key_req.lock().await {
                            let request = KeyRequest {
                                key: spawn_sr.relation.id.clone(),
                                name: name.clone(),
                            };

                            tcp_link
                                .send(request.to_bytes())
                                .await
                                .wrap_msg("Failed to reply to key_request")?;
                        }

                        // Link::close was removed; dropping the link closes
                        // the underlying stream.
                        drop(tcp_link);
                    } else {
                        // handle authentication
                        let x = Authenticated::listen_encrypting(spawn_sr, tcp_link)
                            .await
                            .wrap()?;
                        spawn_sender.send(x).await.wrap()?;
                    }
                    LinkResult::Ok(())
                });
            }
            LinkResult::Err(ErrorKind::Closed.into())
        })
    }
}

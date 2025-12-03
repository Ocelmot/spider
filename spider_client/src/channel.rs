use std::fmt::Debug;

use spider_link::Relation;
use spider_link::link_set::Epoch;
use tracing::debug;
use spider_link::{message::Message, SpiderId2048};
use tokio::sync::mpsc::{unbounded_channel, Sender, UnboundedReceiver};

use crate::error::{ClientError, ClientResult, ErrorKind, Problem, ProblemWrap};
use crate::SpiderClientBuilder;

use crate::client_message::{ClientControl, ClientResponse};


/// A ClientChannel represents a connection to a base. The channel may be
/// unpaired or paired. If it is paired it may be connected or disconnected.
/// Messages can be sent to the base. However, in order for messages to be
/// received, the channel must have reception enabled via enable_recv.
pub struct ClientChannel {
    self_id: SpiderId2048,
    sender: Sender<ClientControl>,
    receiver: Option<UnboundedReceiver<ClientResponse>>,
}

impl ClientChannel {
    pub(super) fn new(id: SpiderId2048, sender: Sender<ClientControl>) -> Self {
        Self {
            self_id: id,
            sender,
            receiver: None,
        }
    }
    pub(super) fn with_receiver(
        id: SpiderId2048,
        sender: Sender<ClientControl>,
        receiver: UnboundedReceiver<ClientResponse>,
    ) -> Self {
        Self {
            self_id: id,
            sender,
            receiver: Some(receiver),
        }
    }

    /// Get the id for the peripheral side of the channel.
    pub fn id(&self) -> &SpiderId2048 {
        &self.self_id
    }

    /// Instruct this client to pair with the Relation.
    /// 
    /// If the client is already paired this will have no effect. To change the
    /// relation this client is paired to, unpair before calling this function.
    pub async fn pair(&self, rel: Relation) ->  ClientResult {
        match self.sender.send(ClientControl::Pair(rel)).await {
            Ok(_) => Ok(()),
            Err(_) => {
                Err(ClientError::new().problem(ErrorKind::Closed))
            },
        }
    }

    /// Causes the client to start the connection
    pub async fn connect(&self) -> ClientResult {
        match self.sender.send(ClientControl::Connect).await {
            Ok(_) => Ok(()),
            Err(_) => {
                Err(ClientError::new().problem(ErrorKind::Closed))
            },
        }
    }

    /// Send a message through the channel to the base.
    pub async fn send(&self, msg: Message) -> ClientResult {
        match self.sender.send(ClientControl::Message(msg, None)).await {
            Ok(_) => Ok(()),
            Err(_) => {
                Err(ClientError::new().problem(ErrorKind::Closed))
            },
        }
    }

    /// Causes the client to stop the connection
    pub async fn disconnect(&self) -> ClientResult {
        match self.sender.send(ClientControl::Disconnect).await {
            Ok(_) => Ok(()),
            Err(_) => {
                Err(ClientError::new().problem(ErrorKind::Closed))
            },
        }
    }

    /// Unpairs the client if it is currently paired.
    pub async fn unpair(&self) -> ClientResult {
        match self.sender.send(ClientControl::Unpair).await {
            Ok(_) => Ok(()),
            Err(_) => {
                Err(ClientError::new().problem(ErrorKind::Closed))
            },
        }
    }

    /// Terminates the entire client, returning the client's state through the
    /// recv channel and callbacks.
    pub async fn terminate(&mut self) -> ClientResult {
        match self.sender.send(ClientControl::Terminate).await {
            Ok(_) => Ok(()),
            Err(_) => {
                Err(ClientError::new().problem(ErrorKind::Closed))
            },
        }
    }

    /// Register a function to be called with all subsequent messages.
    pub async fn set_on_message<F>(&self, cb: Option<F>)
    where
        F: FnMut(&ClientChannel, Message, Epoch) + Send + 'static,
    {
        self.sender
            .send(ClientControl::SetOnMessage(match cb {
                Some(cb) => Some(Box::new(cb)),
                None => None,
            }))
            .await
            .ok();
    }

    /// Register a function to be called when the channel becomes connected.
    pub async fn set_on_connect<F>(&self, cb: Option<F>)
    where
        F: FnMut(&ClientChannel, Epoch) + Send + 'static,
    {
        self.sender
            .send(ClientControl::SetOnConnect(match cb {
                Some(cb) => Some(Box::new(cb)),
                None => None,
            }))
            .await
            .ok();
    }

    /// Register a function to be called when the channel becomes disconnected.
    pub async fn set_on_terminate<F>(&self, cb: Option<F>)
    where
        F: FnMut(SpiderClientBuilder) + Send + 'static,
    {
        self.sender
            .send(ClientControl::SetOnTerminate(match cb {
                Some(cb) => Some(Box::new(cb)),
                None => None,
            }))
            .await
            .ok();
    }

    /// Enable this channel to receive messages from the base.
    /// 
    /// This is disabled by default to avoid the channel filling up
    /// if its messages are not frequently read.
    pub async fn enable_recv(&mut self, set: bool) -> ClientResult {
        if set {
            if let None = self.receiver {
                let (tx, rx) = unbounded_channel();
                if let Err(_) = self.sender.send(ClientControl::AddChannel(tx)).await {
                    return Err(ClientError::new().problem(ErrorKind::Closed));
                }
                self.receiver = Some(rx);
            }
        } else {
            if let Some(rx) = &mut self.receiver {
                rx.close();
                self.receiver = None;
            }
        }
        Ok(())
    }

    /// Receive a message from this channel,
    /// waiting if there is none currently.
    pub async fn recv(&mut self) -> ClientResult<ClientResponse> {
        match &mut self.receiver {
            Some(receiver) => {
                debug!("Has Receiver");
                receiver.recv().await.wrap().problem(ErrorKind::Closed)
            }
            None => {
                debug!("No receiver");
                Err(ClientError::new().msg("ClientChannel has not enabled receiving messages. Call .enable_recv()."))
            }
        }
    }
}

impl Clone for ClientChannel {
    fn clone(&self) -> Self {
        Self {
            self_id: self.self_id.clone(),
            sender: self.sender.clone(),
            receiver: None,
        }
    }
}

impl Debug for ClientChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let recv = match self.receiver {
            Some(_) => "<Messages enabled>",
            None => "<Messages disabled>",
        };
        f.debug_struct("ClientChannel")
            .field("sender", &"<Sender channel>")
            .field("receiver", &recv)
            .finish()
    }
}

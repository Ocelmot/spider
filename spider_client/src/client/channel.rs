use std::fmt::Debug;

use spider_link::{message::Message, SpiderId2048};
use tokio::sync::mpsc::{unbounded_channel, Sender, UnboundedReceiver};

use crate::SpiderClientBuilder;

use super::{ClientControl, ClientResponse};

/// A ClientChannel represents a connection to the paired base.
/// However, the connection may be connected or disconnected.
/// If it is disconnected, it can be reconnected.
/// Messages can be sent to the base. However, in order for messages
/// to be received, the channel must have reception enabled via enable_recv.
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
    pub(super) fn with_receiver(id: SpiderId2048, sender: Sender<ClientControl>, receiver: UnboundedReceiver<ClientResponse>) -> Self {
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

    /// Send a message through the channel to the base.
    pub async fn send(&self, msg: Message) {
        self.sender.send(ClientControl::Message(msg)).await;
    }

    /// Register a function to be called with all subsequent messages.
    pub async fn set_on_message<F>(&self, cb: Option<F>)
    where
        F: FnMut(&ClientChannel, Message) + Send + 'static,
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
        F: FnMut(&ClientChannel) + Send + 'static,
    {
        self.sender
            .send(ClientControl::SetOnConnect(match cb {
                Some(cb) => Some(Box::new(cb)),
                None => None,
            }))
            .await
            .ok();
    }

    /// Register a function to be called when the channel becomes disconneted.
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

    /// Enable this channel to recieve messages from the base.
    /// This is disabled by default to avoid the channel filling up
    /// if its messages are not frequently read.
    pub async fn enable_recv(&mut self, set: bool) {
        if set {
            if let None = self.receiver {
                let (tx, rx) = unbounded_channel();
                self.sender.send(ClientControl::AddChannel(tx)).await;
                self.receiver = Some(rx);
            }
        } else {
            if let Some(rx) = &mut self.receiver {
                rx.close();
                self.receiver = None;
            }
        }
    }

    /// Recieve a message from this channel,
    /// waiting if there is none currently.
    pub async fn recv(&mut self) -> Option<ClientResponse> {
        match &mut self.receiver {
            Some(receiver) => receiver.recv().await,
            None => None,
        }
    }

    /// Request that the connection be terminated.
    pub async fn terminate(&mut self) {
        self.sender.send(ClientControl::Terminate).await;
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
        let recv = match self.receiver{
            Some(_) => "<Messages enabled>",
            None => "<Messages disabled>",
        };
        f.debug_struct("ClientChannel")
            .field("sender", &"<Sender channel>")
            .field("receiver", &recv)
            .finish()
    }
}

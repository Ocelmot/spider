use std::fmt::Debug;

use spider_link::message::Message;
use tokio::sync::mpsc::{unbounded_channel, Sender, UnboundedReceiver};

use crate::SpiderClientBuilder;

use super::{ClientControl, ClientResponse};

pub struct ClientChannel {
    sender: Sender<ClientControl>,
    receiver: Option<UnboundedReceiver<ClientResponse>>,
}

impl ClientChannel {
    pub fn new(sender: Sender<ClientControl>) -> Self {
        Self {
            sender,
            receiver: None,
        }
    }
    pub fn with_receiver(sender: Sender<ClientControl>, receiver: UnboundedReceiver<ClientResponse>) -> Self {
        Self {
            sender,
            receiver: Some(receiver),
        }
    }

    pub async fn send(&self, msg: Message) {
        self.sender.send(ClientControl::Message(msg)).await;
    }

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

    pub async fn recv(&mut self) -> Option<ClientResponse> {
        match &mut self.receiver {
            Some(receiver) => receiver.recv().await,
            None => None,
        }
    }

    pub async fn terminate(&mut self) {
        self.sender.send(ClientControl::Terminate).await;
    }
}

impl Clone for ClientChannel {
    fn clone(&self) -> Self {
        Self {
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

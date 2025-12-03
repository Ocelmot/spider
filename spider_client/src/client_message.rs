use spider_link::{Relation, link_set::Epoch, message::Message};
use tokio::sync::mpsc::UnboundedSender;

use crate::{ClientChannel, SpiderClientBuilder};

pub enum ClientControl {
    Pair(Relation),
    Connect,
    Message(Message, Option<Epoch>),
    Disconnect,
    Unpair,
    Terminate,

    AddChannel(UnboundedSender<ClientResponse>),
    SetOnMessage(Option<Box<dyn FnMut(&ClientChannel, Message, Epoch) + Send>>),
    SetOnConnect(Option<Box<dyn FnMut(&ClientChannel, Epoch) + Send>>),
    SetOnTerminate(Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>),
}

/// The ClientResponse enum represents the possible responses that may be
/// returned from a [ClientChannel]. It includes messages,
/// but also connection events.
#[derive(Debug, Clone)]
pub enum ClientResponse {
    /// The client has received a Relation, and is now paired.
    Paired,

    /// The client has connected to the base it paired to.
    Connected(Epoch),

    /// The client has received a message from the base.
    Message(Message, Epoch),
    
    /// The connection between the client and the base has disconnected.
    /// 
    /// The client will attempt to reconnect, either automatically, or when a
    /// new message is sent.
    Disconnected,
    
    /// The client is now unpaired, either because it was requested, or because
    /// the host denied the connection.
    /// 
    /// The client can be paired to another base and continue operating.
    Unpaired(Relation),

    /// The client has terminated, the current state is returned. No further
    /// processing will be done by this client.
    Terminated(SpiderClientBuilder),
}

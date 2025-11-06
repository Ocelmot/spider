use spider_link::{message::Message, Relation};
use tokio::sync::mpsc::UnboundedSender;

use crate::{ClientChannel, SpiderClientBuilder};

pub enum ClientControl {
    Pair(Relation),
    Connect,
    Message(Message, Option<u64>),
    Disconnect,
    Unpair,
    Terminate,

    AddChannel(UnboundedSender<ClientResponse>),
    SetOnMessage(Option<Box<dyn FnMut(&ClientChannel, Message, u64) + Send>>),
    SetOnConnect(Option<Box<dyn FnMut(&ClientChannel, u64) + Send>>),
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
    Connected(u64),

    /// The client has received a message from the base.
    Message(Message, u64),
    
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

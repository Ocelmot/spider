use spider_link::message::Message;
use tokio::sync::mpsc::UnboundedSender;

use crate::{ClientChannel, SpiderClientBuilder};

pub enum ClientControl {
    Message(Message),
    AddChannel(UnboundedSender<ClientResponse>),
    SetOnMessage(Option<Box<dyn FnMut(&ClientChannel, Message) + Send>>),
    SetOnConnect(Option<Box<dyn FnMut(&ClientChannel) + Send>>),
    SetOnTerminate(Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>),
    SetOnDeny(Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>),
    Terminate,
}

/// The ClientResponse enum represents the possible responses that may be
/// returned from a [ClientChannel]. It includes messages,
/// but also connection events.
#[derive(Debug, Clone)]
pub enum ClientResponse {
    /// The peripheral has recieved a message from the base.
    Message(Message),
    /// The peripheral has connected to the base.
    Connected,
    /// The peripheral has disconnected from the base.
    Disconnected,
    /// The connection was terminated, the current state is returned.
    Terminated(SpiderClientBuilder),
    /// The connection was denied by the base, the current state is returned.
    Denied(SpiderClientBuilder),
}

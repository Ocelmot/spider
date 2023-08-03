use spider_link::message::Message;
use tokio::sync::mpsc::UnboundedSender;

use crate::{ClientChannel, SpiderClientBuilder};

pub enum ClientControl {
    Message(Message),
    AddChannel(UnboundedSender<ClientResponse>),
    SetOnMessage(Option<Box<dyn FnMut(&ClientChannel, Message) + Send>>),
    SetOnConnect(Option<Box<dyn FnMut(&ClientChannel) + Send>>),
    SetOnTerminate(Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>),
    Terminate,
}

#[derive(Debug, Clone)]
pub enum ClientResponse {
    Message(Message),
    Connected,
    Terminated(SpiderClientBuilder),
}

use serde::{Serialize, Deserialize};
use tokio::sync::mpsc::Sender;

use crate::SpiderId;


#[derive(Debug, Clone)]
pub enum SpiderMessage{
    Control(Control),
    Message(Message),
}

#[derive(Debug, Clone)]
pub enum Control{
    Introduction{id: SpiderId, channel: Sender<Message>},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message{
    Introduction{id: u32, as_peripheral: bool},
    Message{msg_type: String, routing: Option<u32>, body: Vec<u8>},
}

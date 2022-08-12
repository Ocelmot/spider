use crate::{Role, SpiderId2048};

use serde::{Serialize, Deserialize};


#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Frame{
	pub data: Vec<u8>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Protocol{
	Introduction{id: SpiderId2048, role: Role},
	Message(Message),
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message{
	Ui,
	Dataset,
	Message{data: Vec<u8>},
}
use crate::{Role, SpiderId2048};

use serde::{Serialize, Deserialize};

mod ui;
pub use crate::message::ui::{
	UiMessage,
	UiPage,
	UiPageManager,
	UiPageList,
	UiPath,

	UiElement,
	UiElementKind,
	UiElementUpdate,
};

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
	Peripheral(PeripheralMessage),
	Ui(UiMessage),
	Dataset,
	Event(EventMessage),
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeripheralMessage{
	

}



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMessage{
	name: String,
	data: Vec<u8>
}
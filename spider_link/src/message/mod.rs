use crate::{Role, SpiderId2048};

use serde::{Deserialize, Serialize};

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

	UiInput,
};

mod dataset;
pub use dataset::{
    DatasetMessage,
    AbsoluteDatasetScope,
    AbsoluteDatasetPath,
    DatasetScope,
    DatasetPath,
    DatasetData,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Frame {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Protocol {
    Introduction { id: SpiderId2048, role: Role },
    Message(Message),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Peripheral(PeripheralMessage),
    Ui(UiMessage),
    Dataset(DatasetMessage),
    Event(EventMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeripheralMessage {

}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMessage {
    name: String,
    data: Vec<u8>,
}

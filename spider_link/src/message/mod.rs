use crate::{Role, SpiderId2048, Relation};

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
    UiElementChange,
    UiElementContent,
    UiElementContentPart,
    UiChildOperations,
    UpdateSummary,

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
    Ui(UiMessage),
    Dataset(DatasetMessage),
    Router(RouterMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouterMessage {
    SendEvent(String, Vec<Relation>, DatasetData),
    Event(String, Relation, DatasetData),
    Subscribe(String),
    Unsubscribe(String),
}

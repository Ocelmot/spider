//! This module contains Message type as well as its variants and
//! operations to manage them.
//! 
//! The main Message variants are UI, Dataset, Router, and Error.
//! Some of these types simply convey some information, like Error,
//! but some also have much more complex operation, like UI.
//! 
//! This module also houses some of the inner types used in the
//! spider protocol.


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

mod router;
pub use router::{
    RouterMessage,
    DirectoryEntry,
    Invite,
};

mod group;
pub use group::{
    GroupMessage,
    GroupId,
    GroupEvent,

    ChangeAnnounce,
    ChangeAck,
    ChangeId,

    Proposal,
    ProposalId,
    ProposalAction,
    ProposalDatasetChange,
};

// mod veilid;
// pub use veilid::{
//     VeilidFrame,
//     FrameManager,
//     VeilidMessage,
// };

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Frame {
    pub data: Vec<u8>,
}

/// The key request is used by a peripheral to get the id and
/// name of the listening base. This struct contains the
/// response to that request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRequest{
    /// The id of the queried base.
    pub key: SpiderId2048,
    /// The human readable name of the base.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Protocol {
    Introduction { id: SpiderId2048, role: Role },
    Message(Message),
}

/// A Message sent to or from a member of the spider network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// The message is a [UiMessage].
    /// Used to control and manage the input of a Ui page
    Ui(UiMessage),

    /// The message is a [DatasetMessage].
    /// Used to manage the data in the datasets
    Dataset(DatasetMessage),

    /// The message is a [RouterMessage].
    /// Used to route arbitrary data to members of the network
    Router(RouterMessage),

    /// The message is a [GroupMessage].
    /// Used to synchronize the datasets of a group
    Group(GroupMessage),

    /// The message is an error
    Error(String),
}

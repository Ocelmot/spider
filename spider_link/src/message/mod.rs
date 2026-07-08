//! This module contains Message type as well as its variants and
//! operations to manage them.
//! 
//! The main Message variants are UI, Dataset, Router, and Error.
//! Some of these types simply convey some information, like Error,
//! but some also have much more complex operation, like UI.
//! 
//! This module also houses some of the inner types used in the
//! spider protocol.


use crate::{LinkResult, SpiderId2048, error::{ErrorKind, ProblemWrap}};

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

impl KeyRequest {
    /// Converts the KeyRequest into a Vec<u8>
    pub fn to_bytes(&self) -> Vec<u8>{
        let key_bytes = self.key.to_bytes();
        let key_bytes_len = key_bytes.len() as u32;

        let name_bytes = self.name.as_bytes();
        let name_bytes_len = name_bytes.len() as u32;

        let mut out = Vec::with_capacity(4 + 4 + key_bytes_len as usize + name_bytes_len as usize);

        out.extend_from_slice(&name_bytes_len.to_be_bytes());
        out.extend_from_slice(name_bytes);

        out.extend_from_slice(&key_bytes_len.to_be_bytes());
        out.extend_from_slice(key_bytes);
        
        out
    }

    /// Parses the KeyRequest from a slice
    pub fn from_bytes(data: &[u8]) -> LinkResult<Self>{
        let (name_len, remainder) = data.split_at_checked(4).ok_or(ErrorKind::Deserialization)?;
        let name_len = u32::from_be_bytes(name_len.try_into().unwrap());
        let (name, remainder) = remainder.split_at_checked(name_len as usize).ok_or(ErrorKind::Deserialization)?;
        let name = String::from_utf8(name.to_vec()).wrap_problem(ErrorKind::Deserialization)?;

        let (key_len, remainder) = remainder.split_at_checked(4).ok_or(ErrorKind::Deserialization)?;
        let key_len = u32::from_be_bytes(key_len.try_into().unwrap());
        let (key_bytes, _remainder) = remainder.split_at_checked(key_len as usize).ok_or(ErrorKind::Deserialization)?;
        let key = SpiderId2048::from_bytes(key_bytes.try_into().unwrap());

        Ok(Self { key, name })
    }
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

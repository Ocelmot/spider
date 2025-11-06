use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{Relation, SpiderId2048};

use super::{DatasetData, DatasetPath};

mod id;
pub use id::GroupId;

mod change;
pub use change::{ChangeAck, ChangeAnnounce, ChangeId};

mod proposal;
pub use proposal::{Proposal, ProposalAction, ProposalDatasetChange, ProposalId};

/// GroupMessage manages the synchronization of data between members of a group.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GroupMessage {
    /// A message with a type, sender, and some data to send to all members
    /// of the group.
    /// These messages are not synchronized with respect to each other.
    Broadcast {
        /// The group to which to broadcast
        group_id: GroupId,
        /// The sender of the broadcast
        sender: Relation,
        /// The type of message being sent
        msg_type: String,
        /// The data the message carries
        data: DatasetData,
    },

    // Synchronization messages
    /// A request for a change with the given id.
    /// This is how the changes spread through the system.
    /// Changes are spread via pull, acks are spread via push
    /// (and pull on upkeep)
    Sync {
        /// The group to request updates about
        group_id: GroupId,
        /// The id of the being requested.
        change_id: ChangeId,
    },

    /// A request for acknowledgments of a sequence number.
    SyncAck {
        /// The group to request updates about
        group_id: GroupId,
        /// The sequence number being requested.
        seq_num: u64,
    },

    /// Inform a node about an announced change.
    Change {
        /// The group the updates apply to
        group_id: GroupId,
        /// The requested change.
        change: ChangeAnnounce,
    },

    /// Inform a node about some change acknowledgements.
    Ack {
        /// The group the updates apply to
        group_id: GroupId,
        /// A list of acknowledgements.
        acks: Vec<ChangeAck>,
    },

    /// Represents a full synchronization of the group's data.
    /// This could be a response when a Sync's last_id is not in a node's
    /// list of recent changes.
    State {
        /// The group to be synchronized.
        group_id: GroupId,
        /// The sequence number of the sent state.
        seq_num: u64,
        /// The members of this group
        members: HashSet<SpiderId2048>,
        /// The datasets of this group
        #[serde_as(as = "Vec<(_, _)>")]
        datasets: HashMap<DatasetPath, Vec<DatasetData>>,
        /// The metadata for the group
        #[serde_as(as = "Vec<(_, _)>")]
        metadata: HashMap<DatasetPath, f32>,
    },

    // ===== These are the messages to handle peripheral communication
    /// Create a new group with the given id. If the group already exists,
    /// no action will be taken. Also subscribes to the newly created group.
    Create(GroupId),
    
    /// Invite the recipient to the group. This contains the group id and the
    /// member set. The recipient must synchronize following this.
    Invite(GroupId, HashSet<SpiderId2048>),

    /// Subscribe to events from this group
    Subscribe(GroupId),

    /// An event has occurred in the group
    GroupEvent(GroupId, GroupEvent),

    // These are to be replaced with the GroupEvent system
    // /// Subscribe this peripheral to raw proposal actions of the group
    // SubscribeProposalAction(GroupId),
    // /// Inform a peripheral of a raw proposal action
    // ProposalAction(GroupId, ProposalAction),
    // /// Subscribe this peripheral to the accepted proposals of the group
    // SubscribeProposal(GroupId),
    // /// Inform a peripheral of an accepted proposal of the group
    // Proposal(GroupId, Proposal),
    
    /// Stop recieving updates to this group
    Unsubscribe(GroupId),
    /// The peripheral asks the base to propose to the group
    Propose(GroupId, Proposal),
    /// The peripheral asks the base to vote on a proposal
    Vote(GroupId, ProposalId, bool),
    /// Get one of the group's datasets
    GetDataset(GroupId, DatasetPath),
    /// Return one of the group's datasets to the peripheral
    Dataset(GroupId, DatasetPath, Vec<DatasetData>),
}

/// A GroupEvent informs a subscribed peripheral of events that occur in the group.
/// This includes new proposals, votes on proposals,
/// and the successful resolution of accepted proposals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GroupEvent {
    /// This proposal is new to the group, and is under consideration.
    Propose(Proposal),

    /// This is a vote on one of the group's proposals.
    Vote(ProposalId, bool),

    /// This proposal has been accepted into the group's data.
    Resolution(Proposal),
}

impl From<ProposalAction> for GroupEvent {
    fn from(value: ProposalAction) -> Self {
        match value {
            ProposalAction::Propose(proposal) => GroupEvent::Propose(proposal),
            ProposalAction::Vote(proposal_id, vote) => GroupEvent::Vote(proposal_id, vote),
        }
    }
}

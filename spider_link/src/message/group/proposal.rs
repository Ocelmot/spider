use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

use crate::{message::{DatasetData, DatasetPath}, SpiderId2048};

/// A unique itentifier for a [Proposal]
pub type ProposalId = BigUint;

/// A message indicating an action related to a [Proposal].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProposalAction {
    /// Add a new proposal to the list of proposals under consideration.
    Propose(Proposal),

    /// Record a vote on a proposal in favor or against.
    /// If the votes are enough to succeed, the changes in the proposal are
    /// committed to the groups dataset. If there are enough votes against the
    /// proposal to prevent it from passing, it is removed from the list and
    /// ignored. If the proposal neither has enough votes to pass, nor enough
    /// votes to prevent it from passing, The vote is recorded and saved.
    Vote(ProposalId, bool),
}

impl ProposalAction {
    /// Calculates and returns a hash of the ProposalAction
    pub fn sha256(&self) -> String {
        let input = serde_json::to_string(self).unwrap();
        sha256::digest(input)
    }

    /// Generate a string representation. (Currently uses JSON)
    pub fn to_string(&self) -> String{
        serde_json::to_string(self).unwrap()
    }
}

/// Describes the change to the group's dataset in the proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProposalDatasetChange {
    /// Add a new member to the group
    AddMember(SpiderId2048),
    /// Remove this member from the group
    RemoveMember(SpiderId2048),
    /// Change the number of votes required to modify this dataset
    SetMetadata(f32),
    /// Reset the number of votes required to modify this dataset
    ClearMetadata,
    /// Set the entry at the given location
    SetData(usize, DatasetData),
    /// Add data to the end of the dataset
    AppendData(DatasetData),
    /// Delete the item in the dataset at the index
    RemoveData(usize),
}

/// A Proposal describes a possible change in a group's data.
/// The change could modify the data, the metadata, or even the set of users.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    id: ProposalId,
    /// Change some dataset in some way (always relative to the group root)
    dataset: DatasetPath,
    /// The change to be applied to the group's dataset.
    dataset_change: ProposalDatasetChange,
}

impl Proposal {
    /// Create a new Proposal acting on the dataset referred to by the
    /// DatasetPath, and performing the change described by the
    /// ProposalDatasetChange.
    pub fn propose(dataset: DatasetPath, dataset_change: ProposalDatasetChange) -> Self {
        let data: Vec<u8> = [
            serde_json::to_vec(&dataset).unwrap(),
            serde_json::to_vec(&dataset_change).unwrap(),
        ]
        .concat();
        let id = sha256::digest(data.as_slice());
        let id = BigUint::parse_bytes(id.as_bytes(), 16).unwrap();
        Self {
            id,
            dataset,
            dataset_change,
        }
    }

    /// Returns a reference to the id of the Proposal.
    pub fn id(&self) -> &ProposalId {
        &self.id
    }

    /// Returns a reference to the DatasetPath referred to by this Proposal.
    pub fn dataset_path(&self) -> &DatasetPath {
        &self.dataset
    }

    /// Returns a reference to the ProposalDatasetChange
    /// referred to by this Proposal.
    pub fn dataset_change(&self) -> &ProposalDatasetChange {
        &self.dataset_change
    }

    /// Split the Proposal into its constituant path and change
    pub fn to_parts(self) -> (DatasetPath, ProposalDatasetChange) {
        (self.dataset, self.dataset_change)
    }

    /// Verify that the Proposal's id matches the changes described by the
    /// DatasetPath and ProposalDatasetChange.
    pub fn verify(&self) -> bool {
        let data: Vec<u8> = [
            serde_json::to_vec(&self.dataset).unwrap(),
            serde_json::to_vec(&self.dataset_change).unwrap(),
        ]
        .concat();
        let id = sha256::digest(data.as_slice());
        let id = BigUint::parse_bytes(id.as_bytes(), 16).unwrap();
        self.id == id
    }
}

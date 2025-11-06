use std::{
    cmp::{max, min},
    collections::{hash_map::Entry, BTreeMap, BTreeSet, HashMap, HashSet},
    ops::Bound::{Excluded, Unbounded},
    path::PathBuf,
};

use tracing::info;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use spider_link::{
    message::{
        ChangeAck, ChangeAnnounce, ChangeId, DatasetData, DatasetPath, GroupEvent, GroupId,
        GroupMessage, Message, Proposal, ProposalAction, ProposalDatasetChange, ProposalId,
    },
    Relation, SpiderId2048,
};
use tokio::fs::{create_dir_all, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::processor::link::ProcessorLink;

use num_bigint::BigUint;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    // Unsaved properties
    /// The path from which the group was loaded, and to which it will be saved.
    #[serde(skip)]
    load_path: Option<PathBuf>,

    // Saved properties
    /// The id of the group
    id: GroupId,

    /// The members of the group
    members: HashSet<SpiderId2048>,

    /// Metadata describes the requirements for various kinds
    /// of proposals to pass.
    #[serde_as(as = "Vec<(_, _)>")]
    metadata: HashMap<DatasetPath, f32>,

    /// Holds the hashes of the datasets to ensure integrity.
    /// Can be used to enumerate the datasets
    #[serde_as(as = "Vec<(_, _)>")]
    datasets: HashMap<DatasetPath, String>,

    /// The set of known changes, ordered by id
    #[serde_as(as = "Vec<(_, _)>")]
    changes: HashMap<ChangeId, ChangeAnnounce>,

    /// Ordered set of ids of changes that may not have any acks yet.
    /// E.g. changes this node wants to send, but are not ackd yet.
    pending_changes: BTreeSet<ChangeId>,

    /// Index of change acknowlegements.
    /// This is a map from the sequence number to
    /// a map from [ChangeId]s to acknowlegements.
    #[serde_as(as = "Vec<(_, Vec<(_, _)>)>")]
    acks: BTreeMap<u64, BTreeMap<ChangeId, HashSet<ChangeAck>>>,

    /// The most recently accepted sequence number.
    current_seq_num: u64,

    /// The currently accepted change from this node.
    candidate: Option<ChangeId>,

    /// The active proposals the group is considering.
    /// The value is a tuple of the proposal and a map from member ids to thier
    /// votes.
    #[serde_as(as = "Vec<(_, (_, Vec<(_,_)>))>")]
    proposals: HashMap<ProposalId, (Proposal, HashMap<SpiderId2048, bool>)>,
}

impl Group {
    pub fn new(path: PathBuf, id: GroupId, member_vec: Vec<SpiderId2048>) -> Self {
        let mut members = HashSet::new();

        for rel in member_vec {
            members.insert(rel);
        }
        Self {
            load_path: Some(path),

            id,
            members,
            metadata: HashMap::new(),
            datasets: HashMap::new(),
            changes: HashMap::new(),
            pending_changes: BTreeSet::new(),
            acks: BTreeMap::new(),
            current_seq_num: 0,
            candidate: None,
            proposals: HashMap::new(),
        }
    }

    pub async fn load_dir(path: PathBuf) -> Option<Self> {
        let mut file = File::open(&path.join("group.json")).await.ok()?;
        let mut buf = String::new();
        file.read_to_string(&mut buf).await.ok()?;

        let mut ret: Group = serde_json::from_str(&buf).ok()?;
        ret.load_path = Some(path);

        Some(ret)
    }

    pub async fn save(&self) {
        let path = self.load_path.as_ref().unwrap().join("group.json");
        create_dir_all(path.parent().unwrap()).await;
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(path)
            .await
            .expect("failed to open file for writing");
        let data = serde_json::to_vec(self).expect("failed to serialize group");
        file.write_all(&data).await.expect("failed to write_all");
        let _ = file.set_len(data.len().try_into().unwrap()).await;
    }

    pub fn id(&self) -> &GroupId {
        &self.id
    }

    pub fn get_proposals(&self) -> Vec<Proposal> {
        let proposals = self
            .proposals
            .iter()
            .map(|(_, (v, _))| v)
            .cloned()
            .collect();
        proposals
    }

    fn insert_ack(&mut self, ack: ChangeAck) {
        let seq_num = ack.seq_num().clone();
        let ack_id = ack.id().clone();
        match self.acks.get_mut(&seq_num) {
            Some(seq_num_acks) => {
                match seq_num_acks.get_mut(&ack_id) {
                    Some(ack_set) => {
                        ack_set.insert(ack);
                    }
                    None => {
                        let mut ack_set = HashSet::new();
                        ack_set.insert(ack);
                        seq_num_acks.insert(ack_id.clone(), ack_set);
                    }
                }
            }
            None => {
                // create a new btreemap and insert,
                // if this is not before the earliest ack.
                if ack.seq_num() >= self.oldest_valid_seq_num() {
                    let mut ack_set = HashSet::new();
                    ack_set.insert(ack);

                    let mut tree = BTreeMap::new();
                    tree.insert(ack_id.clone(), ack_set);
                    self.acks.insert(seq_num, tree);
                }
            }
        }
        // If the ack is for a future change, add the change to pending changes.
        if seq_num > self.current_seq_num {
            self.pending_changes.insert(ack_id);
        }
    }

    fn somecast_qty(&self) -> usize {
        let limit = min(
            self.members.len() as usize,
            max(5usize, (self.members.len() + 1).ilog2() as usize),
        );
        limit
    }

    /// Send the group message to a subset of the group's members.
    async fn somecast_group_msg(&self, pl: &mut ProcessorLink, msg: GroupMessage) {
        let rels: Vec<Relation> = self
            .members
            .iter()
            .cloned()
            .map(|e| Relation::peer_from_id(e))
            .collect();
        let msg = Message::Group(msg);
        pl.somecast_message(rels, self.somecast_qty(), msg).await;
    }

    fn oldest_valid_seq_num(&self) -> u64 {
        match self.acks.first_key_value() {
            Some((accepted, _)) => *accepted,
            None => self.current_seq_num + 1,
        }
    }

    /// Handler for responding to a [GroupMessage::Sync] request by sending updated changes.
    pub async fn handle_sync(
        &self,
        sender: &mut ProcessorLink,
        rel: Relation,
        change_id: ChangeId,
    ) {
        info!("Responding to sync request for change: {:?}", change_id);

        // reply with the change, if known.
        if let Some(change) = self.changes.get(&change_id) {
            let msg = GroupMessage::Change {
                group_id: self.id,
                change: change.clone(),
            };
            sender.send_message(rel, Message::Group(msg)).await;
        }
    }

    /// Handler for [GroupMessage::SyncAck] request.
    /// Responds by sending a list of acks for the requested sequence number.
    pub async fn handle_sync_ack(&self, sender: &mut ProcessorLink, rel: Relation, seq_num: u64) {
        info!("handling sync_ack request for seq_num {}", seq_num);
        if let Some(changes) = self.acks.get(&seq_num) {
            print!("Found acks: ");
            let mut sync_list = Vec::new();
            for (_, ack_set) in changes {
                sync_list.extend(ack_set.iter().cloned());
            }
            info!("{:?}", sync_list.len());
            let msg = GroupMessage::Ack {
                group_id: self.id,
                acks: sync_list,
            };
            let msg = Message::Group(msg);
            sender.send_message(rel.clone(), msg).await;
        }
        // If the seq_num is before all known changes, send a sync message instead.
        if seq_num < self.oldest_valid_seq_num() {
            let mut datasets = HashMap::new();
            for (path, _) in &self.datasets{
                let dataset = self.load_dataset(&path).await;
                datasets.insert(path.clone(), dataset);
            }
            let msg = GroupMessage::State {
                group_id: self.id.clone(),
                seq_num: self.current_seq_num,
                members: self.members.clone(),
                datasets,
                metadata: self.metadata.clone(),
            };
            let msg = Message::Group(msg);
            sender.send_message(rel, msg).await;
        }
    }

    /// Handle the reciept of a change previously requested.
    pub async fn handle_change(&mut self, pl: &mut ProcessorLink, change: ChangeAnnounce) -> Vec<GroupEvent> {
        if change.validate_with(&self.members) {
            self.changes.insert(change.id().clone(), change);
            self.process_reaction(pl).await
        }else{
            Vec::new()
        }
    }

    pub async fn handle_acks(
        &mut self,
        sender: &mut ProcessorLink,
        acks: Vec<ChangeAck>,
    ) -> Vec<GroupEvent> {
        info!("Handling {} akcs...", acks.len());
        for ack in acks {
            info!("Ack: seq_num={}", ack.seq_num());
            // Check that the ack came from the group.
            if !ack.verify(&self.members) {
                info!("Rejecting ack as invalid!");
                continue;
            }

            // insert the ack into the ack index
            self.insert_ack(ack);
        }

        // Check if the group needs to respond to the change in Acks
        self.process_reaction(sender).await
    }

    /// Process this node's reaction to changes in the group state.
    /// It checks the current state of the node and determines if it needs to
    /// accept any changes, or send any acknowlegements.
    async fn process_reaction(&mut self, pl: &mut ProcessorLink) -> Vec<GroupEvent> {
        let mut events = Vec::new();
        loop {
            // check if there is a candidate.
            // If there is a candidate, determine if that candidate has been
            // accepted or can no longer be accepted.
            if let Some(acceptee) = self.test_candidate_acceptability() {
                // this candidate should be accepted
                let new_events = self.accept_candidate_change(pl, acceptee).await;
                events.extend(new_events);
            }

            // try to select a new candidate
            let new_candidate = self.select_new_candidate(pl).await;
            // If a new candidate was not selected,
            // there is nothing left to react to.
            if !new_candidate {
                break;
            }
        }
        events
    }

    /// Test if the candidate should remain or be cleared.
    /// Does nothing if there was no candidate.
    /// If the candidate is accepted, it clears the candidate variable.
    /// If the candidate is no longer a potentially accpeted change,
    /// it clears the candidate variable.
    /// Returns the id of the change that should be accepted.
    fn test_candidate_acceptability(&mut self) -> Option<ChangeId> {
        if let Some(candidate_id) = &self.candidate {
            // get the current ack list
            let current_acks = self.acks.get(&(self.current_seq_num + 1)).expect(
                "if there is a candidate, there should be atleast the ack for that candidate",
            );

            // tally the acks of this id
            let candidate_tally = current_acks
                .get(candidate_id)
                .expect("candidates should be ack'd by this node itself")
                .len();

            // if it passes 75% the candidate has passed, clear and return the id.
            // This change has been accepted, it is no longer pending.
            let tally_limit = (0.75 * self.members.len() as f32) as usize;
            if candidate_tally > tally_limit {
                let c_id = candidate_id.clone();
                self.candidate = None;
                self.pending_changes.remove(&c_id);
                self.current_seq_num += 1;
                return Some(c_id);
            }

            // otherwise, tally the acks for the changes ahead of it
            let front_tally = current_acks
                .range((Unbounded, Excluded(candidate_id)))
                .fold(0, |acc, (_, e)| acc + e.len());

            // if they pass 25% this candidate cannot pass, clear the candidate.
            if (self.members.len() - front_tally) <= tally_limit {
                self.candidate = None;
            }
            None
        } else {
            None
        }
    }

    /// If there is no candidate, choose a new candidate if possible.
    /// If a new candidate is chosen, the acknowlegement is also sent.
    async fn select_new_candidate(&mut self, pl: &mut ProcessorLink) -> bool {
        info!("Candidate = {:?}", self.candidate);
        // Only choose a new candidate if we do not have one already
        if let None = self.candidate {
            // Only choose a new candidate if there is a pending change available.
            if let Some(id) = self.pending_changes.first() {
                // Only choose a new candidate if we know about the change itself.
                // if let Some(change)
                if self.changes.contains_key(&id) {
                    // Set new candidate
                    self.candidate = Some(id.clone());

                    // Send the acknowlegement
                    let us = pl.state().self_relation().await;
                    let ack = ChangeAck::acknowledge(us, id.clone(), self.current_seq_num + 1u64);
                    let msg = GroupMessage::Ack {
                        group_id: self.id,
                        acks: vec![ack.clone()],
                    };
                    self.somecast_group_msg(pl, msg).await;

                    // Store ack
                    self.insert_ack(ack);
                    true
                } else {
                    // Request this change from some of the other nodes.
                    let msg = GroupMessage::Sync {
                        group_id: self.id,
                        change_id: id.clone(),
                    };
                    self.somecast_group_msg(pl, msg).await;
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Apply this change to the group's datasets.
    /// Return any [GroupEvent]s that have occurred.
    async fn accept_candidate_change(
        &mut self,
        pl: &mut ProcessorLink,
        candidate_id: ChangeId,
    ) -> Vec<GroupEvent> {
        let candidate_change = self
            .changes
            .get(&candidate_id)
            .expect("candidate_id should be in self.changes");
        let mut events = Vec::new();
        events.push(candidate_change.proposal().clone().into());

        match candidate_change.proposal() {
            ProposalAction::Propose(proposal) => {
                let proposal_id = proposal.id().clone();
                self.proposals
                    .insert(proposal_id, (proposal.clone(), HashMap::new()));
            }
            ProposalAction::Vote(proposal_id, vote) => {
                let entry = self.proposals.entry(proposal_id.clone());
                if let Entry::Occupied(mut entry) = entry {
                    info!("Vote entry exists");
                    let (proposal, votes) = entry.get_mut();
                    if !self.members.contains(candidate_change.signatory()) {
                        info!("Vote from non member");
                        return Vec::new(); // a vote must come from a member
                    }
                    votes.insert(candidate_change.signatory().clone(), *vote);

                    info!("Tallying");
                    // tally the votes, and check if this proposal should resolve
                    let metadata_limit =
                        self.metadata.get(proposal.dataset_path()).unwrap_or(&0.75);
                    let mut votes_for = 0;
                    let mut votes_against = 0;
                    for (_, vote) in votes {
                        if *vote {
                            votes_for += 1;
                        } else {
                            votes_against += 1;
                        }
                    }

                    if (votes_for as f32 / self.members.len() as f32) > *metadata_limit {
                        // If this vote has passed, make the change
                        let (proposal, _) = entry.remove();
                        info!(
                            "Vote caused proposal to pass {}/{}, applying...",
                            votes_for,
                            self.members.len()
                        );
                        self.apply_proposal(pl, proposal.clone()).await;
                        events.push(GroupEvent::Resolution(proposal));
                    } else if (votes_against as f32 / self.members.len() as f32)
                        > (1.0 - metadata_limit)
                    {
                        info!(
                            "Vote caused proposal to fail {}/{} ({} against), removing",
                            votes_for,
                            self.members.len(),
                            votes_against
                        );
                        // If this vote can no longer pass, erase it
                        entry.remove();
                    } else {
                        info!(
                            "Vote caused proposal to neither pass or fail: {}/{}",
                            votes_for,
                            self.members.len()
                        );
                    }
                }
            }
        }

        events
    }

    /// Handle the reception of a State message.
    /// This is a full sync of the group.
    pub async fn handle_state(
        &mut self,
        rel: Relation,
        seq_num: u64,
        members: HashSet<SpiderId2048>,
        datasets: HashMap<DatasetPath, Vec<DatasetData>>,
        metadata: HashMap<DatasetPath, f32>,
    ) {
        info!("Handling state");
        // verify this state is valid/needed
        if !self.members.contains(&rel.id) {
            return;
        }
        if self.current_seq_num > seq_num {
            return;
        }
        // TODO: Check if we recieve similar state messages from other members
        // for verification purposes.

        self.current_seq_num = seq_num;
        self.members = members;
        self.metadata = metadata;

        self.datasets.clear();
        for (path, dataset) in datasets {
            let hash = self.save_dataset(&path, dataset).await;
            self.datasets.insert(path, hash);
        }

        self.save().await;
    }

    async fn apply_proposal(&mut self, pl: &mut ProcessorLink, proposal: Proposal) {
        info!("Applying the proposal ");
        let (path, change) = proposal.to_parts();
        match change {
            ProposalDatasetChange::AddMember(id) => {
                info!("Adding a member");
                self.members.insert(id.clone());
                // Also send the invite to this new member
                let rel = Relation::peer_from_id(id.clone());
                let msg = GroupMessage::Invite(self.id.clone(), self.members.clone());
                let msg = Message::Group(msg);
                pl.send_message(rel, msg).await;
            }
            ProposalDatasetChange::RemoveMember(id) => {
                self.members.remove(&id);
            }
            ProposalDatasetChange::SetMetadata(vote_limit) => {
                self.metadata.insert(path, vote_limit);
            }
            ProposalDatasetChange::ClearMetadata => {
                self.metadata.remove(&path);
            }
            ProposalDatasetChange::SetData(index, item) => {
                let mut dataset = self.load_dataset(&path).await;
                if let Some(element) = dataset.get_mut(index) {
                    *element = item;
                }
                let hash = self.save_dataset(&path, dataset).await;
                self.datasets.insert(path, hash);
            }
            ProposalDatasetChange::AppendData(item) => {
                info!("Appending data");
                let mut dataset = self.load_dataset(&path).await;
                dataset.push(item);
                let hash = self.save_dataset(&path, dataset).await;
                self.datasets.insert(path, hash);
            }
            ProposalDatasetChange::RemoveData(index) => {
                let mut dataset = self.load_dataset(&path).await;
                dataset.remove(index);
                let hash = self.save_dataset(&path, dataset).await;
                self.datasets.insert(path, hash);
            }
        }
    }

    pub async fn propose(&mut self, pl: &mut ProcessorLink, proposal: Proposal) -> Vec<GroupEvent> {
        // Create change announcement to represent the new proposal
        let self_rel = pl.state().self_relation().await;
        let pa = ProposalAction::Propose(proposal);
        let change = ChangeAnnounce::announce_new(self_rel, pa);

        // Add change to set of changes
        let change_id = change.id().clone();
        self.changes.insert(change_id.clone(), change);

        // Add change id to candidates.
        self.pending_changes.insert(change_id);

        // Trigger group reaction
        self.process_reaction(pl).await
    }

    pub async fn vote(
        &mut self,
        pl: &mut ProcessorLink,
        proposal: ProposalId,
        vote: bool,
    ) -> Vec<GroupEvent> {
        // Create change announce to represent the vote to cast
        let self_rel = pl.state().self_relation().await;
        let pa = ProposalAction::Vote(proposal, vote);
        let change = ChangeAnnounce::announce_new(self_rel, pa);

        // Add change to set of changes
        let change_id = change.id().clone();
        self.changes.insert(change_id.clone(), change);

        // Add change id to candidates.
        self.pending_changes.insert(change_id);

        // Trigger group reaction
        self.process_reaction(pl).await
    }

    /// Request a sync for this group
    pub async fn sync(&self, pl: &mut ProcessorLink) {
        let msg = GroupMessage::SyncAck {
            group_id: self.id.clone(),
            seq_num: self.current_seq_num + 1,
        };

        self.somecast_group_msg(pl, msg).await;
    }

    /// Request a full sync for this group
    pub async fn full_sync(&self, pl: &mut ProcessorLink) {
        let msg = GroupMessage::SyncAck {
            group_id: self.id.clone(),
            // There should be no change for seq_num 0, meaning that it is
            // before all other changes and will trigger a state message
            // to be sent.
            seq_num: 0,
        };

        self.somecast_group_msg(pl, msg).await;
    }

    pub async fn load_dataset(&self, path: &DatasetPath) -> Vec<DatasetData> {
        // If there is no entry in self.datasets,
        // the data on the disk should be ignored.
        if !self.datasets.contains_key(path) {
            return Vec::new();
        }

        // Convert to file path
        let base = &self.load_path.clone().unwrap();
        let mut p = base.join("datasets");
        for item in path.parts() {
            p.push(item);
        }
        p.set_extension("dat");
        // create directories above file
        create_dir_all(p.parent().unwrap()).await.unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(p)
            .await
            .unwrap();
        let mut data: String = String::new();
        file.read_to_string(&mut data).await;
        if data.len() == 0 {
            data = String::from("[]");
        }
        serde_json::from_str(&data).unwrap()
    }

    pub async fn save_dataset(&self, path: &DatasetPath, dataset: Vec<DatasetData>) -> String {
        // Convert to file path
        let base = &self.load_path.clone().unwrap();
        let mut p = base.join("datasets");
        for item in path.parts() {
            p.push(item);
        }
        p.set_extension("dat");
        info!("Path: {}", p.display());
        // create directories above file
        create_dir_all(p.parent().unwrap()).await.unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(p)
            .await
            .unwrap();

        let data = serde_json::to_string(&dataset).unwrap();

        file.write_all(data.as_bytes()).await;
        file.set_len(data.len().try_into().unwrap()).await;

        sha256::digest(data)
    }

    pub fn print(&self) {
        // Print group state
        info!("Current seq_num {}", self.current_seq_num);
        info!("changes count {}", self.changes.len());
        info!("pending changes count {}", self.pending_changes.len());
        let candidate = match &self.candidate {
            Some(candidate_id) => {
                let mut s = String::new();

                s += &(candidate_id % BigUint::from(1000000u32)).to_string();

                s += " seq: ";
                s += &(self.current_seq_num+1).to_string();
                Some(s)
            }
            None => None,
        };
        info!("candidate: {:?}", candidate);
        info!("Pending changes ({})", self.pending_changes.len());
        for change in &self.pending_changes {
            info!(
                "id: {}",
                (change % BigUint::from(1000000u32)).to_string()
            );
        }
    }
}

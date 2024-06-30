use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use log::{error, info};
use num_bigint::BigUint;
use spider_link::message::{GroupEvent, GroupId, GroupMessage, Message};
use spider_link::Relation;
use tokio::fs::create_dir;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task::{JoinError, JoinHandle};

mod message;
pub use message::GroupProcessorMessage;

mod group;
use group::Group;

use crate::config::SpiderConfig;
use crate::state_data::StateData;

use super::link::ProcessorLink;

pub(crate) struct GroupProcessor {
    sender: Sender<GroupProcessorMessage>,
    handle: JoinHandle<()>,
}

impl GroupProcessor {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorLink) -> Self {
        let (group_sender, group_receiver) = channel(50);
        let processor = GroupProcessorState::new(config, state, sender, group_receiver);
        let handle = processor.start();
        Self {
            sender: group_sender,
            handle,
        }
    }

    pub async fn send(
        &mut self,
        message: GroupProcessorMessage,
    ) -> Result<(), SendError<GroupProcessorMessage>> {
        self.sender.send(message).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

pub(crate) struct GroupProcessorState {
    config: SpiderConfig,
    state: StateData,
    sender: ProcessorLink,
    receiver: Receiver<GroupProcessorMessage>,

    groups: HashMap<GroupId, Group>,
    pending_groups: HashMap<GroupId, Group>,
    subscribers: HashMap<GroupId, HashSet<Relation>>,
}

impl GroupProcessorState {
    pub fn new(
        config: SpiderConfig,
        state: StateData,
        sender: ProcessorLink,
        receiver: Receiver<GroupProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
            receiver,

            groups: HashMap::new(),
            pending_groups: HashMap::new(),
            subscribers: HashMap::new(),
        }
    }

    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            // Load the groups!
            info!("Loading groups....");
            let group_path = self.config.group_path();
            if !group_path.exists() {
                create_dir(&group_path).await;
            }
            self.load_groups_from(group_path).await;

            // Process incoming messages
            loop {
                let msg = match self.receiver.recv().await {
                    Some(msg) => msg,
                    None => break,
                };
                match msg {
                    GroupProcessorMessage::PublicMessage(rel, msg) => {
                        self.handle_public_message(rel, msg).await
                    }
                    GroupProcessorMessage::Upkeep => {
                        self.handle_upkeep().await;
                    }
                }
            }
        });
        handle
    }

    async fn load_groups_from(&mut self, dir: PathBuf) {
        for item in dir.read_dir().expect("to be able to read directory") {
            match item {
                Ok(path) => {
                    if let Some(group) = Group::load_dir(path.path()).await {
                        self.groups.insert(group.id().clone(), group);
                    }
                }
                Err(e) => {
                    error!("Failed to load group: {}", e);
                }
            }
        }
    }

    async fn handle_public_message(&mut self, rel: Relation, msg: GroupMessage) {
        match msg {
            GroupMessage::Broadcast {
                group_id,
                sender,
                msg_type,
                data,
            } => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    // group.broadcast()
                }
            }
            GroupMessage::Sync {
                group_id,
                change_id,
            } => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    group.handle_sync(&mut self.sender, rel, change_id).await;
                }
            }
            GroupMessage::SyncAck { group_id, seq_num } => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    group.handle_sync_ack(&mut self.sender, rel, seq_num).await;
                }
            }
            GroupMessage::Change { group_id, change } => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    let events = group.handle_change(&mut self.sender, change).await;
                    self.handle_subscribers(group_id, events).await;
                }
            }
            GroupMessage::Ack { group_id, acks } => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    let events = group.handle_acks(&mut self.sender, acks).await;
                    self.handle_subscribers(group_id, events).await;
                }
            }
            GroupMessage::State {
                group_id,
                seq_num,
                members,
                datasets,
                metadata,
            } => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    group
                        .handle_state( rel, seq_num, members, datasets, metadata)
                        .await;
                    group.sync(&mut self.sender).await
                } else if let Entry::Occupied(entry) = self.pending_groups.entry(group_id) {
                    let (group_id, mut group) = entry.remove_entry();
                    group.handle_state(rel, seq_num, members, datasets, metadata).await;
                    self.groups.insert(group_id, group);
                }
            }

            // Peripheral message handling
            GroupMessage::Create(group_id) => {
                info!("Creating group");
                if !self.groups.contains_key(&group_id) {
                    let mut path = self.config.group_path();
                    path.push(group_id.as_simple().to_string());
                    let us = self.state.self_relation().await.relation;
                    let group = Group::new(path, group_id, vec![us.id]);
                    group.save().await;
                    self.groups.insert(group.id().clone(), group);
                }
            }
            // This node has been invited
            GroupMessage::Invite(group_id, members) => {
                info!("recieving invite");
                // Create the group as long as it does not exist
                if !self.groups.contains_key(&group_id) && !self.pending_groups.contains_key(&group_id) {
                    let us = self.state.self_relation().await.relation;
                    // Test that our id is in the set of members
                    if members.contains(&us.id) {
                        let mut path = self.config.group_path();
                        path.push(group_id.as_simple().to_string());
                        let group = Group::new(path, group_id, members.into_iter().collect());
                        // The full_sync here represents this node accepting the invitation to the group.
                        // TODO: move this such that the user or a peripheral can choose to accept the membership.
                        group.full_sync(&mut self.sender).await;
                        self.pending_groups.insert(group_id, group);
                    }
                }
            }

            GroupMessage::Subscribe(group_id) => {
                if self.groups.contains_key(&group_id) {
                    let entry = self.subscribers.entry(group_id);
                    let subscribers = entry.or_default();
                    subscribers.insert(rel);
                }
            }
            GroupMessage::GroupEvent(_, _) => {} // Base sends this, not recieve
            GroupMessage::Unsubscribe(group_id) => {
                // Remove from proposal subscribers list
                let entry = self.subscribers.entry(group_id);
                if let Entry::Occupied(mut entry) = entry {
                    let subscribers = entry.get_mut();
                    subscribers.remove(&rel);
                    if subscribers.is_empty() {
                        entry.remove();
                    }
                }
            }
            GroupMessage::Propose(group_id, proposal) => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    let events = group.propose(&mut self.sender, proposal).await;
                    self.handle_subscribers(group_id, events).await;
                }
            }
            GroupMessage::Vote(group_id, proposal_id, vote) => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    let events = group.vote(&mut self.sender, proposal_id, vote).await;
                    self.handle_subscribers(group_id, events).await;
                }
            }
            GroupMessage::GetDataset(group_id, path) => {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    let dataset = group.load_dataset(&path).await;
                    let msg = GroupMessage::Dataset(group_id, path, dataset);
                    let msg = Message::Group(msg);
                    self.sender.send_message(rel, msg).await;
                }
            }
            GroupMessage::Dataset(_, _, _) => todo!(), // Base sends does not recieve.
        }
        info!("^^^^^^^^^^^^^^^handle public message^^^^^^^^^^^^^^^^^^");
        for (_, group) in &self.groups {
            group.print();
        }
    }

    /// Process the GroupEvents that occur as a reaction to recieved changes
    /// or acknowlegements and notify the subscribed peripherals.
    async fn handle_subscribers(&mut self, group_id: GroupId, events: Vec<GroupEvent>) {
        if let Some(subscribers) = self.subscribers.get(&group_id) {
            info!("Processing GroupEvents...");
            let rels: Vec<Relation> = subscribers.iter().cloned().collect();
            for event in events {
                info!("GroupEvent occurred: {:?}", event);
                let msg = GroupMessage::GroupEvent(group_id, event);
                let msg = Message::Group(msg);
                self.sender.multicast_message(rels.clone(), msg).await;
            }
        }
    }

    async fn handle_upkeep(&mut self) {
        let self_id = self.state.self_id().await.as_big_uint();
        info!(
            "This node's id: {}",
            (self_id % BigUint::from(1000000u32)).to_string()
        );
        for group in self.groups.values() {
            group.sync(&mut self.sender).await;
            group.save().await;
        }
    }
}

use std::collections::{HashMap, HashSet};

use phf::{phf_set, Set};
use spider_link::{
    message::{DirectoryEntry, Message, RouterMessage, UiInput},
    Relation,
};

use crate::{
    error::SpiderResult,
    processor::{link::ProcessorLink, message::ProcessorMessage, ui::UiProcessorMessage},
};

use super::RouterProcessorMessage;

static SYSTEM_PROPERTIES: Set<&'static str> = phf_set! {
    "nickname",
    "blocked",
};

static SELF_PROPERTIES: Set<&'static str> = phf_set! {
    "name",
};

pub enum LinkApproval {
    Blocked,
    Unknown,
    Allowed,
}

pub struct Directory {
    pl: ProcessorLink,
    entries: HashMap<Relation, DirectoryEntry>,
    subscribers: HashSet<Relation>,
}

impl Directory {
    pub async fn load_directory(mut pl: ProcessorLink) -> Self {
        let entries = pl.state().load_directory().await;

        for (_, entry) in &entries {
            set_directory_entry_ui(&mut pl, entry).await;
        }

        Self {
            pl: pl.clone(),
            entries,
            subscribers: HashSet::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_link_approved(&self, rel: &Relation) -> LinkApproval {
        if let Some(entry) = self.entries.get(rel) {
            if let Some(blocked) = entry.get("blocked") {
                if blocked == "true" {
                    LinkApproval::Blocked
                } else {
                    LinkApproval::Allowed
                }
            } else {
                LinkApproval::Allowed
            }
        } else {
            LinkApproval::Unknown
        }
    }

    pub async fn add_identity(&mut self, rel: &Relation) -> SpiderResult {
        if !self.entries.contains_key(rel) {
            let new_entry = DirectoryEntry::new(rel.clone());
            self.entries.insert(rel.clone(), new_entry.clone());
            self.pl.state().save_directory(&self.entries).await;

            // set/update setting entry
            set_directory_entry_ui(&mut self.pl, &new_entry).await;

            let msg = RouterMessage::AddIdentity(new_entry);
            self.message_subscribers(msg).await;
        }
        Ok(())
    }

    pub fn get_self_property(&self, rel: &Relation, key: &str) -> Option<&String> {
        if !SELF_PROPERTIES.contains(&key) {
            return None; // only specified keys are allowed to be set by clients
        }

        let entry = self.entries.get(rel)?;
        entry.get(key)
    }

    pub async fn set_self_property<S, T>(&mut self, rel: Relation, key: S, value: T)
    where
        S: Into<String>,
        T: Into<String>,
    {
        let key = key.into();
        if !SELF_PROPERTIES.contains(&key) {
            return; // only specified keys are allowed to be set by clients
        }
        let value = value.into();

        let ident = match self.entries.get_mut(&rel) {
            Some(entry) => {
                if Some(&value) == entry.get(&key) {
                    return; // value is already set
                }
                entry
            }
            None => {
                self.entries
                    .insert(rel.clone(), DirectoryEntry::new(rel.clone()));
                self.entries
                    .get_mut(&rel)
                    .expect("entry should still exist")
            }
        };

        ident.set(key, value);

        // set/update setting entry
        set_directory_entry_ui(&mut self.pl, &ident).await;

        let msg = RouterMessage::AddIdentity(ident.clone());
        self.message_subscribers(msg).await;
    }

    pub fn get_system_property(&self, rel: &Relation, key: &str) -> Option<&String> {
        if !SYSTEM_PROPERTIES.contains(&key) {
            return None; // only specified keys are allowed to be set by the system
        }

        let entry = self.entries.get(rel)?;
        entry.get(key)
    }

    pub fn get_entry(&self, rel: &Relation) -> Option<&DirectoryEntry> {
        self.entries.get(rel)
    }

    pub async fn modify_entry(&mut self, rel: &Relation, func: impl FnOnce(&mut DirectoryEntry)) {
        if let Some(mut ident) = self.entries.get_mut(rel) {
            func(&mut ident);

            set_directory_entry_ui(&mut self.pl, &ident).await;
            let msg = RouterMessage::AddIdentity(ident.clone());
            self.message_subscribers(msg).await;
        }
    }

    pub async fn modify_or_insert_entry(&mut self, rel: &Relation, func: impl FnOnce(&mut DirectoryEntry)) {
        // If the identity does not exist, this adds it.
        self.add_identity(rel).await;
        if let Some(mut ident) = self.entries.get_mut(rel) {
            func(&mut ident);

            set_directory_entry_ui(&mut self.pl, &ident).await;
            let msg = RouterMessage::AddIdentity(ident.clone());
            self.message_subscribers(msg).await;
        }
    }

    pub async fn set_system_property<S, T>(&mut self, rel: Relation, key: S, value: T)
    where
        S: Into<String>,
        T: Into<String>,
    {
        let key = key.into();
        if !SYSTEM_PROPERTIES.contains(&key) {
            return; // only specified keys are allowed to be set by the system
        }
        let value = value.into();

        let ident = match self.entries.get_mut(&rel) {
            Some(entry) => {
                if Some(&value) == entry.get(&key) {
                    return; // value is already set
                }
                entry
            }
            None => {
                self.entries
                    .insert(rel.clone(), DirectoryEntry::new(rel.clone()));
                self.entries
                    .get_mut(&rel)
                    .expect("entry should still exist")
            }
        };

        ident.set(key, value);

        // set/update setting entry
        set_directory_entry_ui(&mut self.pl, &ident).await;

        let msg = RouterMessage::AddIdentity(ident.clone());
        self.message_subscribers(msg).await;
    }

    pub async fn remove_identity(&mut self, rel: &Relation) {
        if let None = self.entries.remove(rel) {
            return; // if there was no value, dont update listeners
        }

        let msg = RouterMessage::RemoveIdentity(rel.clone());
        self.message_subscribers(msg).await;

        // remove entry
        clear_directory_entry_ui(&mut self.pl, rel).await
    }

    pub async fn add_subscriber(&mut self, rel: Relation) {
        self.subscribers.insert(rel.clone());

        for (_, directory_entry) in &self.entries {
            let msg = RouterMessage::AddIdentity(directory_entry.clone());
            let msg = Message::Router(msg);
            self.pl.send_message(rel.clone(), msg).await;
        }
    }

    pub fn remove_subscriber(&mut self, rel: &Relation) {
        self.subscribers.remove(rel);
    }

    pub async fn message_subscribers(&self, msg: RouterMessage) -> SpiderResult {
        for subscriber in &self.subscribers {
            let msg = Message::Router(msg.clone());
            self.pl.send_message(subscriber.clone(), msg).await;
        }
        Ok(())
    }

    pub fn approve_message(&self, _rel: &Relation, _msg: &Message) -> bool {
        // for now
        true
    }

    pub async fn upkeep(&mut self) {
        // save modified directory entries
        self.pl.state().save_directory(&self.entries).await;
    }
}

async fn set_directory_entry_ui(pl: &mut ProcessorLink, entry: &DirectoryEntry) {
    let rel = entry.relation();
    let sig = rel.sig();
    let title = format!("{:?}: {}", rel.role, sig);

    let nickname = match entry.get("nickname") {
        Some(nickname) => nickname.clone(),
        None => String::from("-"),
    };
    let name = match entry.get("name") {
        Some(name) => {
            format!("({name})")
        }
        None => String::new(),
    };
    let label = format!("{} {}", nickname, name);

    let msg = UiProcessorMessage::SetSetting {
        header: "Directory".into(),
        title,
        inputs: vec![
            ("text".into(), label),
            ("textentry".into(), "Rename".into()),
            ("button".into(), "Remove".into()),
        ],
        cb: |e| {
            let rel = serde_json::from_str(e.data()).unwrap();
            match e.input() {
                UiInput::Click => {
                    // only button will send click
                    let router_msg = RouterProcessorMessage::ClearDirectoryEntry(rel);
                    let msg = ProcessorMessage::RouterMessage(router_msg);
                    Some(msg)
                }
                UiInput::Text(name) => {
                    // only textentry will send text
                    let router_msg = RouterProcessorMessage::SetNickname(rel, name.clone());
                    let msg = ProcessorMessage::RouterMessage(router_msg);
                    Some(msg)
                }
            }
        },
        data: serde_json::to_string(rel).unwrap(),
    };
    pl.send_ui(msg).await;
}

async fn clear_directory_entry_ui(pl: &mut ProcessorLink, rel: &Relation) {
    let sig = rel.sig();
    let title = format!("{:?}: {}", rel.role, sig);

    let msg = UiProcessorMessage::RemoveSetting {
        header: "Directory".into(),
        title,
    };
    pl.send_ui(msg).await;
}

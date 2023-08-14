
use phf::{Set, phf_set};
use spider_link::{message::{RouterMessage, Message, DirectoryEntry}, Relation};

use crate::processor::{ui::UiProcessorMessage, message::ProcessorMessage};

use super::{RouterProcessorState, RouterProcessorMessage};


static SYSTEM_PROPERTIES: Set<&'static str> = phf_set! {
    "nickname",
    "blocked"
};

static SELF_PROPERTIES: Set<&'static str> = phf_set! {
    "name"
};


// Directory Functionality
impl RouterProcessorState{
    pub(crate) async fn init_directory_functions(&mut self){
        // load directory from state
        self.directory = self.state.load_directory().await;

        // if directory is empty, then allow a single ui connection
        if self.directory.is_empty() {
            self.should_approve_ui.send_replace(true);
        }

        // create setting listing for directory entries
        for (_, entry) in self.directory.clone(){
            self.set_directory_setting(entry).await;
        }
    }

    pub(crate) async fn handle_subscribe_directory(&mut self, rel: Relation){
        // add subscriber
        self.directory_subscribers.insert(rel.clone());
        // send subscriber current directory
        for (_, directory_entry) in &self.directory{
            let msg = RouterMessage::AddIdentity(directory_entry.clone());
            let msg = Message::Router(msg);
            self.sender.send_message(rel.clone(), msg).await;
        }
    }
    pub(crate) async fn handle_unsubscribe_directory(&mut self, rel: Relation){
        self.directory_subscribers.remove(&rel);
    }

    pub(crate) async fn clear_directory_entry_handler(&mut self, rel: Relation) {
        // remove from directory
        self.remove_identity(&rel).await;

        // cancel existing connection
        if let Some(link) = self.links.remove(&rel){
            link.terminate().await;
        }
    }
}

// Operation functions
impl RouterProcessorState{
    pub(crate) async fn add_identity(&mut self, rel: Relation){
        if !self.directory.contains_key(&rel) {
            let new_entry = DirectoryEntry::new(rel.clone());
            self.directory.insert(rel.clone(), new_entry.clone());
            self.state.save_directory(&self.directory).await;

            let msg = RouterMessage::AddIdentity(new_entry.clone());
            self.message_dir_subscribers(msg).await;

            // set/update setting entry
            self.set_directory_setting(new_entry).await;
        }
    }

    pub(crate) async fn set_identity_self(&mut self, rel: Relation, key: String, value: String){
        if !SELF_PROPERTIES.contains(&key) {
            return; // only specified keys are allowed to be set by clients
        }

        let ident = match self.directory.get_mut(&rel){
            Some(entry) => {
                if Some(&value) == entry.get(&key){
                    return; // value is already set
                }
                entry
            },
            None => {
                self.directory.insert(rel.clone(), DirectoryEntry::new(rel.clone()));
                self.directory.get_mut(&rel).expect("entry should still exist")
            },
        };

        ident.set(key, value);
        let updated_entry = ident.clone();

        let msg = RouterMessage::AddIdentity(ident.clone());
        self.message_dir_subscribers(msg).await;

        // set/update setting entry
        self.set_directory_setting(updated_entry).await;
    }

    pub(crate) async fn set_identity_system(&mut self, rel: Relation, key: String, value: String){
        if !SYSTEM_PROPERTIES.contains(&key) {
            return; // only specified keys are allowed to be set by the system
        }

        let ident = match self.directory.get_mut(&rel){
            Some(entry) => {
                if Some(&value) == entry.get(&key){
                    return; // value is already set
                }
                entry
            },
            None => {
                self.directory.insert(rel.clone(), DirectoryEntry::new(rel.clone()));
                self.directory.get_mut(&rel).expect("entry should still exist")
            },
        };
        
        ident.set(key, value);
        let updated_entry = ident.clone();

        let msg = RouterMessage::AddIdentity(ident.clone());
        self.message_dir_subscribers(msg).await;

        // set/update setting entry
        self.set_directory_setting(updated_entry).await;
    }

    pub(crate) async fn remove_identity(&mut self, rel: &Relation){
        if let None = self.directory.remove(rel){
            return; // if there was no value, dont update listeners
        }

        let msg = RouterMessage::RemoveIdentity(rel.clone());
        self.message_dir_subscribers(msg).await;

        // remove entry
        let sig = rel.id.to_base64();
        let sig: String = sig.chars().skip(sig.len().saturating_sub(15)).collect();
        let title = format!("{:?}: {}", rel.role, sig);

        let msg = UiProcessorMessage::RemoveSetting {
            header: "Directory".into(),
            title,
        };
        self.sender.send_ui(msg).await;
    }
}

// Utility functions
impl RouterProcessorState{
    pub(crate) async fn message_dir_subscribers(&mut self, msg: RouterMessage){
        for subscriber in &self.directory_subscribers{
            let msg = Message::Router(msg.clone());
            self.sender.send_message(subscriber.clone(), msg).await;
        }
    }

    pub(crate) async fn set_directory_setting(&mut self, entry: DirectoryEntry){
        let rel = entry.relation();
        let sig = rel.id.to_base64();
        let sig: String = sig.chars().skip(sig.len().saturating_sub(15)).collect();
        let title = format!("{:?}: {}", rel.role, sig);

        let nickname = match entry.get("nickname") {
            Some(nickname) => {
                nickname.clone()
            },
            None => String::from("-"),
        };
        let name = match entry.get("name"){
            Some(name) => {
                format!("({name})")
            },
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
            cb: |idx, name, input, data|{
                let rel = serde_json::from_str(data).unwrap();
                match input{
                    spider_link::message::UiInput::Click => {
                        // only button will send click
                        let router_msg = RouterProcessorMessage::ClearDirectoryEntry(rel);
                        let msg = ProcessorMessage::RouterMessage(router_msg);
                        Some(msg)
                    },
                    spider_link::message::UiInput::Text(name) => {
                        // only textentry will send text
                        let router_msg = RouterProcessorMessage::SetNickname(rel, name);
                        let msg = ProcessorMessage::RouterMessage(router_msg);
                        Some(msg)
                    },
                }
            },
            data: serde_json::to_string(rel).unwrap(),
        };
        self.sender.send_ui(msg).await;
    }
}

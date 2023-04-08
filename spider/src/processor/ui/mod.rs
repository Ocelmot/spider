use std::collections::{BTreeSet, HashMap};

use spider_link::{
    message::{Message, UiMessage, UiPageList, UiInput},
    Relation, Role,
};
use tokio::{
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle},
};

use crate::{config::SpiderConfig, state_data::StateData};

use super::sender::ProcessorSender;

mod settings;

mod message;
pub use message::{UiProcessorMessage, SettingCategory, SettingType};

pub(crate) struct UiProcessor {
    sender: Sender<UiProcessorMessage>,
    handle: JoinHandle<()>,
}

impl UiProcessor {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender) -> Self {
        let (ui_sender, ui_receiver) = channel(50);
        let processor = UiProcessorState::new(config, state, sender, ui_receiver);
        let handle = processor.start();
        Self {
            sender: ui_sender,
            handle,
        }
    }

    pub async fn send(
        &mut self,
        message: UiProcessorMessage,
    ) -> Result<(), SendError<UiProcessorMessage>> {
        self.sender.send(message).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

struct UiProcessorState {
    config: SpiderConfig,
    state: StateData,
    sender: ProcessorSender,
    receiver: Receiver<UiProcessorMessage>,

    pages: UiPageList,
    subscribers: BTreeSet<Relation>,
    settings_callbacks: HashMap<String, fn(&mut ProcessorSender, UiInput)>,
}

impl UiProcessorState {
    fn new(
        config: SpiderConfig,
        state: StateData,
        sender: ProcessorSender,
        receiver: Receiver<UiProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
            receiver,

            pages: UiPageList::new(),
            subscribers: BTreeSet::new(),
            settings_callbacks: HashMap::new(),
        }
    }

    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            self.init_settings().await;
            loop {
                let msg = match self.receiver.recv().await {
                    Some(msg) => msg,
                    None => break,
                };

                match msg {
                    UiProcessorMessage::RemoteMessage(rel, msg) => {
                        self.process_remote_message(rel, msg).await
                    }
                    UiProcessorMessage::SetSetting {
                        category,
                        name,
                        setting_type,
                        callback,
                    } => {
                        self.add_setting(category, name, setting_type, callback).await;
                    }
                    UiProcessorMessage::Upkeep => {}
                }
            }
        });
        handle
    }

    async fn process_remote_message(&mut self, rel: Relation, msg: UiMessage) {
        if let Role::Peer = rel.role {
            return; // role is external, cant control ui
        }
        match msg {
            UiMessage::Subscribe => {
                self.subscribers.insert(rel);
            }
            UiMessage::GetPages => {
                let pages = self.pages.clone_page_vec();
                let msg = Message::Ui(UiMessage::Pages(pages));
                self.sender.send_message(rel, msg).await;
            }
            UiMessage::Pages(_) => {} // ignore, (base sends this, doesnt process it)
            UiMessage::GetPage(id) => match self.pages.get_page_mut(&id) {
                Some(page) => {
                    let msg = Message::Ui(UiMessage::Page(page.get_page().clone()));
                    self.sender.send_message(rel, msg).await;
                }
                None => {}
            },
            UiMessage::Page(_) => {} // ignore, (base sends this, doesnt process it)
            UiMessage::UpdateElementsFor(_, _) => {} // ignore, (base sends this, doesnt process it)
            UiMessage::InputFor(peripheral_id, element_id, input) => {
                // if this is for the settings page, put it there
                if self.state.self_id().await == peripheral_id {
                    self.settings_input(&element_id, input).await;
                }else{
                    // recieve an input from the ui and route it to the peripheral
                    let msg = Message::Ui(UiMessage::Input(element_id, input));
                    let rel = Relation {
                        role: Role::Peripheral,
                        id: peripheral_id,
                    };
                    self.sender.send_message(rel, msg).await;
                }
            }

            UiMessage::SetPage(mut page) => {
                page.set_id(rel.id); // ensure that recieved page uses peripheral's id
                self.pages.upsert_page(page.clone());
                let subscribers: Vec<Relation> = self.subscribers.iter().cloned().collect();

                let msg = Message::Ui(UiMessage::Page(page.clone()));
                self.sender.multicast_message(subscribers, msg).await;
            }
            UiMessage::ClearPage => {}
            UiMessage::UpdateElements(updates) => {
                // get this manager, apply the updates, forward to clients
                match self.pages.get_page_mut(&rel.id) {
                    Some(mgr) => {
                        mgr.apply_changes(updates.clone());
                        // send to clients here
                        let msg = UiMessage::UpdateElementsFor(
                            rel.id.clone(),
                            updates.clone(),
                        );
                        self.ui_to_subscribers(msg).await;
                    }
                    None => {} // no page to update
                }
            }
            UiMessage::Input(..) => {} // ignore, (base sends this, doesnt process it)
        }
    }


}

// Utility functions
impl UiProcessorState {
    pub(crate) async fn ui_to_subscribers(&mut self, msg: UiMessage){
        let subscribers: Vec<Relation> = self.subscribers.iter().cloned().collect();

        let msg = Message::Ui(msg);
        self.sender.multicast_message(subscribers, msg).await;
    }
}
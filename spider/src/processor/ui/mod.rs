use std::collections::{BTreeSet, HashMap, HashSet};

use spider_link::{
    message::{Message, UiMessage, UiPageList, UiInput, AbsoluteDatasetPath, UiElementUpdate, UiPageManager, UiChildOperations, UpdateSummary, DatasetData},
    Relation, Role,
};
use tokio::{
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle},
};

use crate::{config::SpiderConfig, state_data::StateData};

use super::{sender::ProcessorSender, dataset::DatasetProcessorMessage, message::ProcessorMessage};

mod settings;

mod message;
pub use message::{UiProcessorMessage};

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
    dataset_subscriptions: HashMap<AbsoluteDatasetPath, isize>,

    // Settings properties

    settings_callbacks: HashMap<String, (HashMap<String, usize>, Vec<(String, fn(u32, &String, UiInput)->Option<ProcessorMessage>)>)>,
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

            dataset_subscriptions: HashMap::new(),

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
                    UiProcessorMessage::DatasetUpdate(path, dataset) => {
                        // forward dataset updates to clients
                        let msg = UiMessage::Dataset(path, dataset);
                        self.ui_to_subscribers(msg).await;
                    }
                    UiProcessorMessage::SetSetting {
                        header,
                        title,
                        inputs,
                        cb,
                    } => {
                        self.add_setting(header, title, inputs, cb).await;
                    }
                    UiProcessorMessage::RemoveSetting{header, title} => {
                        self.remove_setting(header, title).await;
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
                self.subscribers.insert(rel.clone());
                // send current page list
                let pages = self.pages.clone_page_vec();
                let msg = Message::Ui(UiMessage::Pages(pages));
                self.sender.send_message(rel.clone(), msg).await;
                // send current dataset list
                for i in self.dataset_subscriptions.keys() {
                    let msg = DatasetProcessorMessage::ToUi(rel.clone(), i.clone());
                    self.sender.send_dataset(msg).await;
                }
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
            UiMessage::Dataset(_, _) => {} // ignore, (base sends this, doesnt process it)
            UiMessage::InputFor(peripheral_id, element_id, dataset_ids, input) => {
                // if this is for the settings page, put it there
                if self.state.self_id().await == peripheral_id {
                    self.settings_input(&element_id, dataset_ids, input).await;
                }else{
                    // recieve an input from the ui and route it to the peripheral
                    let msg = Message::Ui(UiMessage::Input(element_id, dataset_ids, input));
                    let rel = Relation {
                        role: Role::Peripheral,
                        id: peripheral_id,
                    };
                    self.sender.send_message(rel, msg).await;
                }
            }

            UiMessage::SetPage(mut page) => {
                page.set_id(rel.id); // ensure that recieved page uses peripheral's id
                let mut summary = UpdateSummary::new();
                // add new page
                summary.add(page.root());
                match self.pages.upsert_page(page.clone()){
                    Some(page) => {
                        // if there was an old page, remove it
                        summary.remove(page.root());
                    },
                    None => {},
                }

                let msg = UiMessage::Page(page.clone());
                self.ui_to_subscribers(msg).await;

                // Handle the summary
                self.update_dataset_summary(summary).await;
            }
            UiMessage::ClearPage => {}
            UiMessage::UpdateElements(updates) => {
                // get this manager, apply the updates, forward to clients
                match self.pages.get_page_mut(&rel.id) {
                    Some(mgr) => {
                        let summary = mgr.apply_changes(updates.clone());
                        // send to clients here
                        let msg = UiMessage::UpdateElementsFor(
                            rel.id.clone(),
                            updates.clone(),
                        );
                        self.ui_to_subscribers(msg).await;
                        // handle summary changes
                        self.update_dataset_summary(summary);
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

    async fn update_dataset_summary(&mut self, summary: UpdateSummary){
        for (path, delta) in summary.dataset_subscriptions() {
            self.update_dataset_subscriptions(path, *delta).await;
        }
    }

    async fn update_dataset_subscriptions(&mut self, path: &AbsoluteDatasetPath, delta: isize){
        match self.dataset_subscriptions.get_mut(path){
            Some(count) => {
                *count += delta;
                if count <= &mut 0 {
                    // unsubscribe + remove entry
                    self.sender.send_dataset(DatasetProcessorMessage::UiUnsubscribe(path.clone())).await;
                    self.dataset_subscriptions.remove(path);
                }
            },
            None => {
                // subscribe if positive delta (negative shouldnt happen)
                if delta > 0 {
                    self.dataset_subscriptions.insert(path.clone(), delta);
                    // send subscription message
                    self.sender.send_dataset(DatasetProcessorMessage::UiSubscribe(path.clone())).await;
                }
            },
        }
    }
}

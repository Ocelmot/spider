use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{config::SpiderConfig, state_data::StateData};

use super::{sender::ProcessorSender, ui::UiProcessorMessage};

mod message;
pub use message::DatasetProcessorMessage;

use spider_link::{
    message::{AbsoluteDatasetPath, AbsoluteDatasetScope, DatasetData, DatasetMessage, Message, UiMessage},
    Relation, SpiderId2048,
};
use tokio::{
    fs::{File, OpenOptions, create_dir_all},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle},
};

#[derive(Debug, PartialEq, Eq, Hash)]
enum DatasetSubscriber {
    Ui,
    Peripheral(SpiderId2048),
}

pub(crate) struct DatasetProcessor {
    sender: Sender<DatasetProcessorMessage>,
    handle: JoinHandle<()>,
}

impl DatasetProcessor {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender) -> Self {
        let (dataset_sender, dataset_receiver) = channel(50);
        let processor = DatasetProcessorState::new(config, state, sender, dataset_receiver);
        let handle = processor.start();
        Self {
            sender: dataset_sender,
            handle,
        }
    }

    pub async fn send(
        &mut self,
        message: DatasetProcessorMessage,
    ) -> Result<(), SendError<DatasetProcessorMessage>> {
        self.sender.send(message).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

pub(crate) struct DatasetProcessorState {
    config: SpiderConfig,
    state: StateData,
    sender: ProcessorSender,
    receiver: Receiver<DatasetProcessorMessage>,

    subscriptions: HashMap<AbsoluteDatasetPath, HashSet<DatasetSubscriber>>,
}

impl DatasetProcessorState {
    pub fn new(
        config: SpiderConfig,
        state: StateData,
        sender: ProcessorSender,
        receiver: Receiver<DatasetProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
            receiver,

            subscriptions: HashMap::new(),
        }
    }

    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            loop {
                let msg = match self.receiver.recv().await {
                    Some(msg) => msg,
                    None => break,
                };

                match msg {
                    DatasetProcessorMessage::PublicMessage(rel, msg) => {
                        self.handle_public_message(rel, msg).await
                    }
                    DatasetProcessorMessage::UiSubscribe(k) => {
                        let is_new = match self.subscriptions.get_mut(&k) {
                            Some(h_set) => h_set.insert(DatasetSubscriber::Ui),
                            None => {
                                let mut h_set = HashSet::new();
                                h_set.insert(DatasetSubscriber::Ui);
                                self.subscriptions.insert(k.clone(), h_set);
                                true
                            }
                        };
                        if is_new {
                            let file_path = self.get_file_path(&k);
                            let dataset = parse_dataset(&file_path).await;
                            self.sender
                                .send_ui(UiProcessorMessage::DatasetUpdate(k, dataset))
                                .await;
                        }
                    }
                    DatasetProcessorMessage::UiUnsubscribe(k) => {
                        match self.subscriptions.get_mut(&k){
                            Some(h_set) => {
                                h_set.remove(&DatasetSubscriber::Ui);
                                // if the set is empty, remove it from the map
                                if h_set.is_empty(){
                                    self.subscriptions.remove(&k);
                                }
                            },
                            None => {
                                // if no set, there was no subscription after all!
                            },
                        }
                    }
                    DatasetProcessorMessage::ToUi(relation, path) => {
                        // send dataset on behalf of the ui processor as a ui update
                        let file_path = self.get_file_path(&path);
                        let dataset = parse_dataset(&file_path).await;
                        let msg = Message::Ui(UiMessage::Dataset(path, dataset));
                        self.sender.send_message(relation, msg).await;
                    }
                    DatasetProcessorMessage::Upkeep => {}
                }
            }
        });
        handle
    }

    async fn handle_public_message(&mut self, rel: Relation, msg: DatasetMessage) {
        match msg {
            DatasetMessage::Subscribe { path } => {
                let path = path.resolve(rel.id.clone());
                // Build value
                let v = DatasetSubscriber::Peripheral(rel.id.clone());
                // Insert, if there is no set already, insert it
                match self.subscriptions.get_mut(&path) {
                    Some(h_set) => {
                        h_set.insert(v);
                    }
                    None => {
                        let mut h_set = HashSet::new();
                        h_set.insert(v);
                        self.subscriptions.insert(path.clone(), h_set);
                    }
                }
                // Reply with dataset
                let file_path = self.get_file_path(&path);
                let dataset = parse_dataset(&file_path).await;
                let msg = Message::Dataset(DatasetMessage::Dataset {
                    path: path.specialize(),
                    data: dataset,
                });
                self.sender.send_message(rel, msg).await;
            }
            DatasetMessage::Append { path, data } => {
                let path = path.resolve(rel.id);
                let file_path = self.get_file_path(&path);
                // open/parse file
                let mut dataset = parse_dataset(&file_path).await;
                // make change
                dataset.push(data);
                // write file
                write_dataset(&file_path, dataset.clone()).await;
                self.message_subscribed(path, &dataset).await;
            }
            DatasetMessage::Extend { path, mut data } => {
                let path = path.resolve(rel.id);
                let file_path = self.get_file_path(&path);
                // open/parse file
                let mut dataset = parse_dataset(&file_path).await;
                // make change
                dataset.append(&mut data);
                // write file
                write_dataset(&file_path, dataset.clone()).await;
                self.message_subscribed(path, &dataset).await;
            }
            DatasetMessage::SetElement { path, data, id } => {
                let path = path.resolve(rel.id);
                let file_path = self.get_file_path(&path);
                // open/parse file
                let mut dataset = parse_dataset(&file_path).await;
                // make change
                // pad
                for _ in dataset.len()..id {
                    dataset.push(DatasetData::Null);
                }
                // set elem
                let elem = dataset
                    .get_mut(id)
                    .expect("dataset should have been extended to length");
                *elem = data;
                // write file
                write_dataset(&file_path, dataset.clone()).await;
                self.message_subscribed(path, &dataset).await;
            }
            DatasetMessage::SetElements { path, data, id } => {
                let path = path.resolve(rel.id);
                let file_path = self.get_file_path(&path);
                // open/parse file
                let mut dataset = parse_dataset(&file_path).await;
                // make change
                // pad
                for _ in dataset.len()..(id + data.len()) {
                    dataset.push(DatasetData::Null);
                }
                // set elems
                for (i, new_elem) in data.into_iter().enumerate() {
                    let elem = dataset
                        .get_mut(id + i)
                        .expect("dataset should have been extended to length");
                    *elem = new_elem;
                }

                // write file
                write_dataset(&file_path, dataset.clone()).await;
                self.message_subscribed(path, &dataset).await;
            }
            DatasetMessage::DeleteElement { path, id } => {
                let path = path.resolve(rel.id);
                let file_path = self.get_file_path(&path);
                // open/parse file
                let mut dataset = parse_dataset(&file_path).await;
                // make change
                if id < dataset.len() {
                    dataset.remove(id);
                }
                // write file
                write_dataset(&file_path, dataset.clone()).await;
                self.message_subscribed(path, &dataset).await;
            }
            DatasetMessage::Empty { path } => {
                let path = path.resolve(rel.id);
                let file_path = self.get_file_path(&path);
                // create empty dataset
                let dataset = vec![];
                // write to file
                write_dataset(&file_path, dataset.clone()).await;
                self.message_subscribed(path, &dataset).await;

            }
            DatasetMessage::Dataset { .. } => {} //base sends this, not recieve (Could use as an assignment operation)
        }
    }

    async fn message_subscribed(&mut self, path: AbsoluteDatasetPath, dataset: &Vec<DatasetData>) {
        match self.subscriptions.get(&path) {
            Some(subscribers) => {
                let mut peripheral_list = Vec::new();
                for subscriber in subscribers {
                    match subscriber {
                        DatasetSubscriber::Ui => {
                            self.sender
                                .send_ui(UiProcessorMessage::DatasetUpdate(
                                    path.clone(),
                                    dataset.clone(),
                                ))
                                .await;
                        }
                        DatasetSubscriber::Peripheral(id) => {
                            peripheral_list.push(Relation {
                                id: id.clone(),
                                role: spider_link::Role::Peripheral,
                            });
                        }
                    }
                }
                let message = DatasetMessage::Dataset {
                    path: path.specialize(),
                    data: dataset.to_vec(),
                };
                let message = spider_link::message::Message::Dataset(message);
                self.sender
                    .multicast_message(peripheral_list, message)
                    .await;
            }
            None => {
                // No subscriptions, no messages
            }
        }
    }

    fn get_file_path(&self, path: &AbsoluteDatasetPath) -> PathBuf {
        let base = &self.config.dataset_path();
        let x = match path.scope() {
            AbsoluteDatasetScope::Peripheral(id) => PathBuf::from_str(&id.sha256()).unwrap(),
            AbsoluteDatasetScope::Public => PathBuf::from_str("public").unwrap(),
        };
        let mut p = base.join(x);
        for item in path.parts() {
            p.push(item);
        }
        p
    }
}

async fn parse_dataset(path: &Path) -> Vec<DatasetData> {
    println!("Path: {}", path.display());
    // create directories above file
    create_dir_all(path.parent().unwrap()).await.unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .await
        .unwrap();
    let mut data: String = String::new();
    file.read_to_string(&mut data).await;
    if data.len() == 0{
        data = String::from("[]");
    }
    serde_json::from_str(&data).unwrap()
}

async fn write_dataset(path: &Path, data: Vec<DatasetData>) {
    // create directories above file
    create_dir_all(path.parent().unwrap()).await.unwrap();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .await
        .unwrap();

    let data = serde_json::to_string(&data).unwrap();

    file.write_all(data.as_bytes()).await;
    file.set_len(data.len().try_into().unwrap()).await;
}

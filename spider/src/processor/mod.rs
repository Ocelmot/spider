use std::io::Error;
use std::path::PathBuf;
use std::{path::Path, time::Duration};

use spider_link::message::Message;
use tokio::{
    sync::mpsc::{channel, Receiver},
    task::{JoinError, JoinHandle},
    time::interval,
};

use crate::{config::SpiderConfig, state_data::StateData};

mod sender;
use sender::ProcessorSender;

mod listener;
use listener::Listener;

mod router;
use router::RouterProcessor;

mod message;
use message::ProcessorMessage;

mod ui;
use ui::{UiProcessor, UiProcessorMessage};

mod peripherals;
use peripherals::PeripheralsProcessor;

mod dataset;
use dataset::DatasetProcessor;

use self::dataset::DatasetProcessorMessage;
use self::peripherals::PeripheralProcessorMessage;
use self::router::RouterProcessorMessage;


pub struct ProcessorBuilder {
    config: Option<SpiderConfig>,
    state: Option<StateData>,
}

impl ProcessorBuilder {
    pub fn new() -> Self {
        Self {
            config: None,
            state: None,
        }
    }

    pub fn config(&mut self, config: SpiderConfig) {
        self.config = Some(config);
    }

    pub fn config_file(&mut self, config_path: &Path) {
        let config = SpiderConfig::from_file(config_path);
        self.config = Some(config);
    }

    pub fn state(&mut self, state: StateData) {
        self.state = Some(state);
    }

    pub fn state_file(&mut self, state_path: &Path) -> Result<(), Error> {
        let state = StateData::load_file(state_path);
        match state {
            Ok(state) => {
                self.state = Some(state);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn is_new(&self) -> bool {
        match &self.state {
            Some(_state) => false,
            None => true,
        }
    }

    pub fn start_processor(self) -> Option<ProcessorHandle> {
        let config = match self.config {
            Some(config) => config,
            None => return None,
        };
        let state = match self.state {
            Some(state) => state,
            None => return None,
        };
        let processor = Processor::new(config, state);
        Some(processor.start())
    }
}

struct Processor {
    state: StateData,
    config: SpiderConfig,
    sender: ProcessorSender,
    receiver: Receiver<ProcessorMessage>,

    listener: Listener,
    router: RouterProcessor,
    peripherals: PeripheralsProcessor,
    ui: UiProcessor,
    dataset_processor: DatasetProcessor,

    print_msg: bool,

    upkeep_interval_handle: JoinHandle<()>,
}

impl Processor {
    fn new(config: SpiderConfig, state: StateData) -> Self {
        // create channel
        let (sender, receiver) = channel(50);
        let sender = ProcessorSender::new(sender);

        // start listener
        let listener = Listener::new(config.clone(), state.clone(), sender.clone());

        // start router
        let router = RouterProcessor::new(config.clone(), state.clone(), sender.clone());

        // start peripherals
        let peripherals = PeripheralsProcessor::new(config.clone(), state.clone(), sender.clone());

        // start ui
        let ui = UiProcessor::new(config.clone(), state.clone(), sender.clone());

        // start datasets
        let dataset_processor = DatasetProcessor::new(config.clone(), state.clone(), sender.clone());

        // start upkeep interval
        let update_channel = sender.clone();
        // let update_state = state.clone();
        let upkeep_interval_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(15));
            loop {
                interval.tick().await;
                update_channel.send(ProcessorMessage::Upkeep).await;
            }
        });

        Self {
            state,
            config,
            sender,
            receiver,

            listener,
            router,
            peripherals,
            ui,
            dataset_processor,

            print_msg: false,

            upkeep_interval_handle,
        }
    }

    fn start(mut self) -> ProcessorHandle {
        let sender = self.sender.clone();

        // start processing
        let handle = tokio::spawn(async move {
            if let Some(path) = &self.config.keyfile_path {
                let path = PathBuf::from(path);
                let data = serde_json::to_string(&self.state.self_id().await).unwrap();
                tokio::fs::write(&*path, data).await;
            }

            let id = self.state.self_id().await.to_base64();

            // Cheat to show the base' client id
            let msg = UiProcessorMessage::SetSetting {
                header: String::from("System"),
                title: id,
                inputs: vec![],
                cb: |_, _, _|{None},
            };
            self.ui.send(msg).await;

            // Button to exit the spider base
            let msg = UiProcessorMessage::SetSetting {
                header: String::from("System"),
                title: String::from("Exit!"),
                inputs: vec![("button".to_string(), "Exit".to_string())],
                cb: |idx, title, input|{
                    std::process::exit(0);
                },
            };
            self.ui.send(msg).await;


            loop {
                let message = self.receiver.recv().await;
                let message = if let Some(message) = message {
                    if self.print_msg {
                        println!("processing message: {:?}", message);
                    }
                    message
                } else {
                    println!("recieved no message, closing...");
                    break; // we did not get a message, all senders have quit, we should too.
                           // we could restart the listener, maybe.
                };

                match message {
                    ProcessorMessage::RemoteMessage(relation, message) => {
                        match message {
                            Message::Ui(msg) => {
                                self.ui
                                    .send(UiProcessorMessage::RemoteMessage(relation, msg))
                                    .await.unwrap();
                            }
                            Message::Dataset(msg) => {
                                self.dataset_processor
                                    .send(DatasetProcessorMessage::PublicMessage(relation, msg))
                                    .await;
                            },
                            Message::Router(msg) => {
                                self.router
                                    .send(RouterProcessorMessage::PeripheralMessage(relation, msg))
                                    .await;
                            }
                        }
                    }
                    ProcessorMessage::RouterMessage(msg) => {
                        self.router.send(msg).await;
                    }
                    ProcessorMessage::UiMessage(msg) => {
                        self.ui.send(msg).await;
                    }
                    
                    ProcessorMessage::DatasetMessage(msg) => {
                        self.dataset_processor.send(msg).await;
                    }
                    ProcessorMessage::PeripheralMessage(msg) => {
                        self.peripherals.send(msg).await;
                    }

                    ProcessorMessage::Upkeep => {
                        self.ui.send(UiProcessorMessage::Upkeep).await;
                        self.dataset_processor.send(DatasetProcessorMessage::Upkeep).await;
                        self.router.send(RouterProcessorMessage::Upkeep).await;
                        self.peripherals.send(PeripheralProcessorMessage::Upkeep).await;
                        self.state.save_file().await;
                    }
                }
            }
        });

        ProcessorHandle { sender, handle }
    }
}

pub struct ProcessorHandle {
    sender: ProcessorSender,
    handle: JoinHandle<()>,
}

impl ProcessorHandle {
    pub(crate) async fn send(&mut self, message: ProcessorMessage) {
        self.sender.send(message).await;
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

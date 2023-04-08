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
use peripherals::Peripherals;

use self::ui::{SettingCategory, SettingType};

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
    peripherals: Peripherals,
    ui: UiProcessor,

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
        let peripherals = Peripherals::new(config.clone(), state.clone(), sender.clone());

        // start ui
        let ui = UiProcessor::new(config.clone(), state.clone(), sender.clone());

        // start datasets

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

            print_msg: true,

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

            // setup setting to disable printing messages
            let msg = UiProcessorMessage::SetSetting {
                category: SettingCategory::Test,
                name: String::from("Exit!"),
                setting_type: SettingType::Button,
                callback: |p, i|{
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
                    ProcessorMessage::RouterMessage(msg) => {
                        self.router.send(msg).await;
                    }
                    ProcessorMessage::UiMessage(ui_message) => {}
                    ProcessorMessage::RemoteMessage(relation, message) => {
                        match message {
                            Message::Ui(ui) => {
                                self.ui
                                    .send(UiProcessorMessage::RemoteMessage(relation, ui))
                                    .await;
                            }
                            Message::Dataset => todo!(),
                            Message::Peripheral(peripheral) => {
                                // self.handle_peripheral(relation, peripheral);
                            }
                            Message::Event(event) => {
                                self.router
                                    .send(router::RouterProcessorMessage::RouteEvent(event))
                                    .await;
                            }
                        }
                    }
                    ProcessorMessage::Upkeep => {
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

use std::io::Error;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::{path::Path, time::Duration};

use spider_link::beacon:: start_beacon_listen_handler_on;
use tracing::info;
use spider_link::message::Message;
use spider_link::Keyfile;
use tokio::fs;
use tokio::{
    sync::mpsc::{channel, Receiver},
    task::{JoinError, JoinHandle},
    time::interval,
};

use crate::error::{SpiderError, SpiderResult};
use crate::{config::SpiderConfig, state_data::StateData};

mod link;
use link::ProcessorLink;

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

mod group;
use group::GroupProcessor;

use self::dataset::DatasetProcessorMessage;
use self::group::GroupProcessorMessage;
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

    pub async fn start_processor(self) -> SpiderResult<ProcessorHandle> {
        let config = match self.config {
            Some(config) => config,
            None => return Err(SpiderError::new().msg("Failed to read config")),
        };
        let state = match self.state {
            Some(state) => state,
            None => return Err(SpiderError::new().msg("Failed to read state")),
        };
        let processor = Processor::new(config, state).await?;
        Ok(processor.start())
    }
}

struct Processor {
    state: StateData,
    config: SpiderConfig,
    sender: ProcessorLink,
    receiver: Receiver<ProcessorMessage>,

    router: RouterProcessor,
    peripherals: PeripheralsProcessor,
    ui: UiProcessor,
    dataset_processor: DatasetProcessor,
    group_processor: GroupProcessor,

    print_msg: bool,

    upkeep_interval_handle: JoinHandle<()>,
}

impl Processor {
    async fn new(config: SpiderConfig, state: StateData) -> SpiderResult<Self> {
        // create channel
        let (sender, receiver) = channel(500);
        let pl = ProcessorLink::new(config.clone(), state.clone(), sender);

        // start router
        let router = RouterProcessor::new(pl.clone()).await?;

        // start beacon
        if config.beacon_enabled() {
            info!("Starting beacon listener.");
            let listen_addr: SocketAddrV4 = config.listen_addr.parse().expect("invalid beacon address");
            let beacon_port = config.beacon_port();
            start_beacon_listen_handler_on(listen_addr.port(), beacon_port);
        }else{
            info!("Beacon listener disabled.");
        }

        // start peripherals
        let peripherals = PeripheralsProcessor::new(config.clone(), state.clone(), pl.clone());

        // start ui
        let ui = UiProcessor::new(config.clone(), state.clone(), pl.clone());

        // start datasets
        let dataset_processor =
            DatasetProcessor::new(config.clone(), state.clone(), pl.clone());

        // start groups
        let group_processor = GroupProcessor::new(config.clone(), state.clone(), pl.clone());

        // start upkeep interval
        let update_channel = pl.clone();
        // let update_state = state.clone();
        let upkeep_interval_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(15));
            loop {
                interval.tick().await;
                update_channel.send(ProcessorMessage::Upkeep).await;
            }
        });

        Ok(Self {
            state,
            config,
            sender: pl,
            receiver,

            router,
            peripherals,
            ui,
            dataset_processor,
            group_processor,

            print_msg: false,

            upkeep_interval_handle,
        })
    }

    fn start(mut self) -> ProcessorHandle {
        let sender = self.sender.clone();

        // start processing
        let handle = tokio::spawn(async move {
            let id = self.state.self_id().await.to_base64();
            fs::write("./id.base64", id).await.expect("failed to write id file");


            if let Some(path) = &self.config.keyfile_path {
                let id = self.state.self_id().await;
                let kf = Keyfile::new(id, None);
                kf.write_to_file(path).await;
            }

            let id = self.state.self_id().await.to_base64();

            // Cheat to show the base' client id
            let msg = UiProcessorMessage::SetSetting {
                header: String::from("System"),
                title: id,
                inputs: vec![],
                cb: |_| None,
                data: String::new(),
            };
            self.ui.send(msg).await;

            // Button to exit the spider base
            let msg = UiProcessorMessage::SetSetting {
                header: String::from("System"),
                title: String::from("Exit!"),
                inputs: vec![("button".to_string(), "Exit".to_string())],
                cb: |_| {
                    std::process::exit(0);
                },
                data: String::new(),
            };
            self.ui.send(msg).await;

            // init setting headers to set the order
            self.ui
                .send(UiProcessorMessage::SetSettingHeader {
                    header: "Pending Connections".into(),
                })
                .await;
            self.ui
                .send(UiProcessorMessage::SetSettingHeader {
                    header: "Peripheral Services".into(),
                })
                .await;
            self.ui
                .send(UiProcessorMessage::SetSettingHeader {
                    header: "Connected Chords".into(),
                })
                .await;
            self.ui
                .send(UiProcessorMessage::SetSettingHeader {
                    header: "Directory".into(),
                })
                .await;

            loop {
                let message = self.receiver.recv().await;
                let message = if let Some(message) = message {
                    if self.print_msg {
                        info!("processing message: {:?}", message);
                    }
                    message
                } else {
                    info!("recieved no message, closing...");
                    break; // we did not get a message, all senders have quit, we should too.
                           // we could restart the listener, maybe.
                };

                match message {
                    ProcessorMessage::RemoteMessage(relation, message) => match message {    
                        Message::Ui(msg) => {
                            self.ui
                                .send(UiProcessorMessage::RemoteMessage(relation, msg))
                                .await
                                .unwrap();
                        }
                        Message::Dataset(msg) => {
                            self.dataset_processor
                                .send(DatasetProcessorMessage::PublicMessage(relation, msg))
                                .await;
                        }
                        Message::Router(msg) => {
                            self.router
                                .send(RouterProcessorMessage::PeripheralMessage(relation, msg))
                                .await;
                        }
                        Message::Group(msg) => {
                            self.group_processor
                                .send(GroupProcessorMessage::PublicMessage(relation, msg))
                                .await;
                        }
                        Message::Error(_) => {}
                    },
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
                        self.dataset_processor
                            .send(DatasetProcessorMessage::Upkeep)
                            .await;
                        self.router.send(RouterProcessorMessage::Upkeep).await;
                        self.peripherals
                            .send(PeripheralProcessorMessage::Upkeep)
                            .await;
                        self.group_processor.send(GroupProcessorMessage::Upkeep).await;
                        self.state.save_file().await;
                    }
                }
            }
        });

        ProcessorHandle { sender, handle }
    }
}

pub struct ProcessorHandle {
    sender: ProcessorLink,
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

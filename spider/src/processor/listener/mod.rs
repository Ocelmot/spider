use tokio::select;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task::{JoinError, JoinHandle};

use crate::processor::ProcessorMessage;
use crate::{config::SpiderConfig, state_data::StateData};
use spider_link::link::Link;

use super::router::RouterProcessorMessage;
use super::sender::ProcessorSender;

mod beacon;
mod message;
pub use message::ListenProcessorMessage;

pub struct ListenerProcessor {
    beacon: JoinHandle<()>,
    sender: Sender<ListenProcessorMessage>,
    handle: JoinHandle<()>,
}

impl ListenerProcessor {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender) -> Self {
        // start beacon
        let beacon = beacon::start_beacon(&config);

        let (listen_sender, listen_receiver) = channel(50);
        let processor = ListenProcessorState::new(config, state, sender, listen_receiver);
        let handle = processor.start();
        Self {
            beacon,
            sender: listen_sender,
            handle,
        }
    }

    pub async fn send(
        &mut self,
        message: ListenProcessorMessage,
    ) -> Result<(), SendError<ListenProcessorMessage>> {
        self.sender.send(message).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

struct ListenProcessorState {
    config: SpiderConfig,
    state: StateData,
    sender: ProcessorSender,
    receiver: Receiver<ListenProcessorMessage>,
}

impl ListenProcessorState {
    pub fn new(
        config: SpiderConfig,
        state: StateData,
        sender: ProcessorSender,
        receiver: Receiver<ListenProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
            receiver,
        }
    }

    pub fn start(mut self) -> JoinHandle<()> {
        let listen_addr = self.config.listen_addr.clone();
        tokio::spawn(async move {
            let self_relation = self.state.self_relation().await;
            let broadcast_name = self.state.name().await.clone();
            let (mut listener, broadcast_setting) = Link::listen(self_relation, listen_addr);
            *broadcast_setting.lock().await = Some(broadcast_name);
            loop {
                select! {
                    // Process Channel
                    msg = self.receiver.recv() => {
                        match msg{
                            Some(msg) => {
                                match msg{
                                    ListenProcessorMessage::SetKeyRequest(key_request) => {
                                        *broadcast_setting.lock().await = key_request;
                                    },
                                    ListenProcessorMessage::Upkeep => {
                                    },
                                }
                            },
                            None => {
                                break; // main processor must have exited
                            },
                        }
                    },
                    // Process Listener
                    link = listener.recv() => {
                        match link {
                            None => {
                                break; // no new link, listener is closed
                            }
                            Some(link) => {
                                let msg = RouterProcessorMessage::NewLink(link);
                                match self.sender.send(ProcessorMessage::RouterMessage(msg)).await {
                                    Ok(_) => {
                                    }
                                    Err(_) => {
                                        break; // channel is closed, processor must also be closed.
                                    }
                                }
                            }
                        }
                    }
                }
            }
        })
    }
}

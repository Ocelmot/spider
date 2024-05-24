use spider_link::{message::Message, Relation};
use tokio::sync::mpsc::{error::SendError, Sender};

use crate::{config::SpiderConfig, state_data::StateData};

use super::{message::ProcessorMessage, router::RouterProcessorMessage, ui::UiProcessorMessage, dataset::DatasetProcessorMessage};

#[derive(Debug, Clone)]
pub struct ProcessorLink {
    config: SpiderConfig,
    state: StateData,
    sender: Sender<ProcessorMessage>,
}

impl ProcessorLink {
    pub(crate) fn new( config: SpiderConfig, state: StateData, sender: Sender<ProcessorMessage>) -> Self {
        Self {
            config,
            state,
            sender,
        }
    }

    pub fn config(&self) -> &SpiderConfig{
        &self.config
    }

    pub fn state(&self) -> &StateData {
        &self.state
    }

    pub(crate) async fn send(&self, msg: ProcessorMessage) -> Result<(), SendError<ProcessorMessage>> {
        self.sender.send(msg).await
    }

    // send message
    pub(crate) async fn send_message(
        &mut self,
        rel: Relation,
        msg: Message,
    ) -> Result<(), SendError<ProcessorMessage>> {
        let msg = RouterProcessorMessage::SendMessage(rel, msg);
        let msg = ProcessorMessage::RouterMessage(msg);
        self.sender.send(msg).await
    }

    /// Send a [Message] to each of the [Relation]s
    pub(crate) async fn multicast_message(
        &mut self,
        rels: Vec<Relation>,
        msg: Message,
    ) -> Result<(), SendError<ProcessorMessage>> {
        let msg = RouterProcessorMessage::MulticastMessage(rels, msg);
        let msg = ProcessorMessage::RouterMessage(msg);
        self.sender.send(msg).await
    }

    // somecast message
    pub(crate) async fn somecast_message(
        &mut self,
        rels: Vec<Relation>,
        limit: usize,
        msg: Message,
    ) -> Result<(), SendError<ProcessorMessage>> {
        let msg = RouterProcessorMessage::SomecastMessage(rels, limit, msg);
        let msg = ProcessorMessage::RouterMessage(msg);
        self.sender.send(msg).await
    }

    // send ui
    pub(crate) async fn send_ui(
        &mut self,
        msg: UiProcessorMessage,
    ) -> Result<(), SendError<ProcessorMessage>> {
        let msg = ProcessorMessage::UiMessage(msg);
        self.sender.send(msg).await
    }

    // send dataset
    pub(crate) async fn send_dataset(
        &mut self,
        msg: DatasetProcessorMessage,
    ) -> Result<(), SendError<ProcessorMessage>> {
        let msg = ProcessorMessage::DatasetMessage(msg);
        self.sender.send(msg).await
    }

}

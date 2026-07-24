use spider_link::{message::Message, Relation};
use tokio::sync::mpsc::Sender;

use crate::{
    config::SpiderConfig,
    error::{ProblemWrap, SpiderResult},
    state_data::StateData,
};

use super::{
    dataset::DatasetProcessorMessage, message::ProcessorMessage, router::RouterProcessorMessage,
    ui::UiProcessorMessage,
};

#[derive(Debug, Clone)]
pub struct ProcessorLink {
    config: SpiderConfig,
    state: StateData,
    sender: Sender<ProcessorMessage>,
}

impl ProcessorLink {
    pub(crate) fn new(
        config: SpiderConfig,
        state: StateData,
        sender: Sender<ProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
        }
    }

    pub fn config(&self) -> &SpiderConfig {
        &self.config
    }

    pub fn state(&self) -> &StateData {
        &self.state
    }

    pub(crate) async fn send(&self, msg: ProcessorMessage) -> SpiderResult {
        self.sender.send(msg).await.wrap()
    }

    // send message
    pub(crate) async fn send_message(&self, rel: Relation, msg: Message) -> SpiderResult {
        let msg = RouterProcessorMessage::SendMessage(rel, msg);
        let msg = ProcessorMessage::RouterMessage(msg);
        self.sender.send(msg).await.wrap()
    }

    /// Send a [Message] to each of the [Relation]s
    pub(crate) async fn multicast_message(
        &self,
        rels: Vec<Relation>,
        msg: Message,
    ) -> SpiderResult {
        let msg = RouterProcessorMessage::MulticastMessage(rels, msg);
        let msg = ProcessorMessage::RouterMessage(msg);
        self.sender.send(msg).await.wrap()
    }

    // somecast message
    pub(crate) async fn somecast_message(
        &self,
        rels: Vec<Relation>,
        limit: usize,
        msg: Message,
    ) -> SpiderResult {
        let msg = RouterProcessorMessage::SomecastMessage(rels, limit, msg);
        let msg = ProcessorMessage::RouterMessage(msg);
        self.sender.send(msg).await.wrap()
    }

    // send ui
    pub(crate) async fn send_ui(&self, msg: UiProcessorMessage) -> SpiderResult {
        let msg = ProcessorMessage::UiMessage(msg);
        self.sender.send(msg).await.wrap()
    }

    // send dataset
    pub(crate) async fn send_dataset(&self, msg: DatasetProcessorMessage) -> SpiderResult {
        let msg = ProcessorMessage::DatasetMessage(msg);
        self.sender.send(msg).await.wrap()
    }
}

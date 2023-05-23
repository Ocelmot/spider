use spider_link::{message::{Message, EventMessage}, Relation};
use tokio::sync::mpsc::{error::SendError, Sender};

use super::{message::ProcessorMessage, router::RouterProcessorMessage, ui::UiProcessorMessage, dataset::DatasetProcessorMessage};

#[derive(Debug, Clone)]
pub struct ProcessorSender {
    sender: Sender<ProcessorMessage>,
}

impl ProcessorSender {
    pub(crate) fn new(sender: Sender<ProcessorMessage>) -> Self {
        Self { sender }
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

    // multicast message
    pub(crate) async fn multicast_message(
        &mut self,
        rels: Vec<Relation>,
        msg: Message,
    ) -> Result<(), SendError<ProcessorMessage>> {
        let msg = RouterProcessorMessage::MulticastMessage(rels, msg);
        let msg = ProcessorMessage::RouterMessage(msg);
        self.sender.send(msg).await
    }

    // route event
    pub(crate) async fn route_event(
        &mut self,
        msg: EventMessage,
    ) -> Result<(), SendError<ProcessorMessage>> {
        let msg = RouterProcessorMessage::RouteEvent(msg);
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

use spider_link::{message::Message, Relation};

use super::{
    dataset::DatasetProcessorMessage,
    listener::ListenProcessorMessage,
    peripherals::PeripheralProcessorMessage,
    router::RouterProcessorMessage,
    ui::UiProcessorMessage,
};

#[derive(Debug)]
pub enum ProcessorMessage {
    RemoteMessage(Relation, Message),
    ListenerMessage(ListenProcessorMessage),
    RouterMessage(RouterProcessorMessage),
    UiMessage(UiProcessorMessage),
    DatasetMessage(DatasetProcessorMessage),
    PeripheralMessage(PeripheralProcessorMessage),
    Upkeep,
}

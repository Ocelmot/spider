use spider_link::{message::Message, Relation};

use super::{
    dataset::DatasetProcessorMessage,
    peripherals::PeripheralProcessorMessage,
    router::RouterProcessorMessage,
    ui::UiProcessorMessage,
    system::SystemProcessorMessage,
};

#[derive(Debug)]
pub enum ProcessorMessage {
    RemoteMessage(Relation, Message),
    RouterMessage(RouterProcessorMessage),
    UiMessage(UiProcessorMessage),
    DatasetMessage(DatasetProcessorMessage),
    PeripheralMessage(PeripheralProcessorMessage),
    SystemMessage(SystemProcessorMessage),
    Upkeep,
}

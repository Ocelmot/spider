

use spider_link::{message::Message, Relation};


use super::{ui::UiProcessorMessage, router::RouterProcessorMessage, dataset::DatasetProcessorMessage, peripherals::PeripheralProcessorMessage};

#[derive(Debug)]
pub enum ProcessorMessage{
    RemoteMessage(Relation, Message),
    RouterMessage(RouterProcessorMessage),
    UiMessage(UiProcessorMessage),
    DatasetMessage(DatasetProcessorMessage),
    PeripheralMessage(PeripheralProcessorMessage),
    Upkeep,
}

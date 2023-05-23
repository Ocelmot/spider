

use spider_link::{message::Message, Relation};


use super::{ui::UiProcessorMessage, router::RouterProcessorMessage, dataset::DatasetProcessorMessage};

#[derive(Debug)]
pub(crate) enum ProcessorMessage{
    RemoteMessage(Relation, Message),
    RouterMessage(RouterProcessorMessage),
    UiMessage(UiProcessorMessage),
    DatasetMessage(DatasetProcessorMessage),
    Upkeep,
}

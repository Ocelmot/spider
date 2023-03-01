

use spider_link::{message::Message, Relation};


use super::{ui::UiProcessorMessage, router::RouterProcessorMessage};

#[derive(Debug)]
pub(crate) enum ProcessorMessage{
    RouterMessage(RouterProcessorMessage),
    UiMessage(UiProcessorMessage),
    RemoteMessage(Relation, Message),
    Upkeep,
}

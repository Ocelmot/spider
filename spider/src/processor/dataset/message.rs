use spider_link::{message::{DatasetMessage, AbsoluteDatasetPath}, Relation};


#[derive(Debug)]
pub enum DatasetProcessorMessage {
    PublicMessage(Relation, DatasetMessage),
    UiSubscribe(AbsoluteDatasetPath),
    Upkeep,
}
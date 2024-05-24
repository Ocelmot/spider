use spider_link::{message::GroupMessage, Relation};

#[derive(Debug)]
pub enum GroupProcessorMessage {
    PublicMessage(Relation, GroupMessage),
    Upkeep,
}

use spider_link::{
    message::{EventMessage, Message},
    Link, Relation,
};

#[derive(Debug)]
pub enum RouterProcessorMessage {
    NewLink(Link),
    SendMessage(Relation, Message),
    MulticastMessage(Vec<Relation>, Message),
    RouteEvent(EventMessage),
    Upkeep,
}

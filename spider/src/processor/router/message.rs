use spider_link::{
    message::{Message, RouterMessage},
    Link, Relation, SpiderId2048,
};

#[derive(Debug)]
pub enum RouterProcessorMessage {
    PeripheralMessage(Relation, RouterMessage),
    
    NewLink(Link),
    SetApprovalCode(String),
    ApproveLink(String),
    DenyLink(String),
    ApprovedLink(Link),

    SendMessage(Relation, Message),
    MulticastMessage(Vec<Relation>, Message),
    SomecastMessage(Vec<Relation>, usize, Message),

    JoinChord(String),
    HostChord(String),
    LeaveChord(String),

    AddrUpdate(SpiderId2048, String),

    SetName(String),
    SetNickname(Relation, String),
    ClearDirectoryEntry(Relation),

    Upkeep,
}

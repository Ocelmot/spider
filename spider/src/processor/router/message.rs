use spider_link::{
    message::{RouterMessage, Message},
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

    JoinChord(String),
    HostChord(String),
    LeaveChord(String),

    AddrUpdate(SpiderId2048, String),

    SetName(String),
    SetNickname(Relation, String),
    ClearDirectoryEntry(Relation),

    Upkeep,
}

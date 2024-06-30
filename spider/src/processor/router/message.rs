use spider_link::{
    message::{Invite, Message, RouterMessage},
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
    /// Set the directory entry for the [Relation] with the key [String] to the value [String]
    SetDirectoryEntry(Relation, String, String), 
    ClearDirectoryEntry(Relation),

    /// Accepts an invite recieved from the user.
    AcceptInvite(Invite),
    /// Revokes an invite, based on a string id that refers to that invite.
    RevokeInvite(String),

    Upkeep,
}

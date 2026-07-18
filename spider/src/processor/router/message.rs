use spider_link::{
    Relation, link_set::Epoch, message::{Message, RouterMessage}
};

#[derive(Debug)]
pub enum RouterProcessorMessage {
    PeripheralMessage(Relation, RouterMessage),

    AddApprovalCode(String),
    ApproveConnection(Relation),
    DenyConnection(Relation),

    Connected(Relation, Epoch),
    UnapprovedMessage(Relation, Message),
    Disconnected(Relation),
    LinkReaderClosed(Relation),

    SendMessage(Relation, Message),
    MulticastMessage(Vec<Relation>, Message),
    SomecastMessage(Vec<Relation>, usize, Message),

    /// Sets the name the base presents to other members of the network
    SetName(String),
    /// Sets a nickname for another member of the network
    SetNickname(Relation, String),

    /// Set the directory entry for the [Relation] with the key [String] to the value [String]
    SetDirectoryEntry(Relation, String, String),

    ClearDirectoryEntry(Relation),

    /// Revokes an invite, based on a string id that refers to that invite.
    RevokeInvite(String),

    Upkeep,
}

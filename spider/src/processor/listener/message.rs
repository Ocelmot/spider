#[derive(Debug)]
pub enum ListenProcessorMessage {
    SetKeyRequest(Option<String>),

    Upkeep,
}

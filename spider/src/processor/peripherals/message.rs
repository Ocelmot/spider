

#[derive(Debug)]
pub enum PeripheralProcessorMessage {
    Install(String),
    Start(String),
    Stop(String),
    Remove(String),
    Upkeep,
}

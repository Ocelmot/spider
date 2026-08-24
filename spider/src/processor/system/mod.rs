use tokio::{sync::mpsc::{Sender, channel}, task::{JoinError, JoinHandle}};

use crate::{error::{ErrorKind, ProblemWrap, SpiderResult}, processor::link::ProcessorLink};

mod message;
pub use message::SystemProcessorMessage;

mod processor_state;
use processor_state::SystemProcessorState;

pub(crate) struct SystemProcessor{
    sender: Sender<SystemProcessorMessage>,
    handle: JoinHandle<()>,
}

impl SystemProcessor {
    pub fn new(pl: ProcessorLink)-> Self{
        let (system_sender, peripheral_receiver) = channel(50);
        let processor = SystemProcessorState::new(pl, peripheral_receiver);
        let handle = processor.start();


        Self{
            sender: system_sender,
            handle
        }
    }

    pub async fn send(
        &mut self,
        message: SystemProcessorMessage,
    ) -> SpiderResult {
        self.sender.send(message).await.wrap_problem(ErrorKind::Stopped)
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}




use crate::{config::SpiderConfig, state_data::StateData};

use super::sender::ProcessorSender;




pub(crate) struct Peripherals{
    sender: ProcessorSender,
}

impl Peripherals {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender)-> Self{
        Self{
            sender,
        }
    }
}
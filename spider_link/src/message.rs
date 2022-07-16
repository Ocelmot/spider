use serde::{Serialize, Deserialize};



#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum SpiderProtocol{
	Introduction{id: u32, as_peripheral: bool},
	Message(SpiderMessage)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SpiderMessage{


}
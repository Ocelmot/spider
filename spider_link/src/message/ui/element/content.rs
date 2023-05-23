use std::vec;

use serde::{Deserialize, Serialize};

use crate::message::DatasetData;


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiElementContent{
    parts: Vec<UiElementContentPart>
}


impl UiElementContent{
    pub fn new() -> Self{
        Self{
            parts: Vec::new()
        }
    }

    pub fn new_text(text: String) -> Self{
        let parts = vec![UiElementContentPart::Text(text)];
        Self {
            parts
        }
    }

    pub fn new_data(property: String) -> Self{
        let parts = vec![UiElementContentPart::Data(vec![property])];
        Self {
            parts
        }
    }

    pub fn add_part(&mut self, part: UiElementContentPart){
        self.parts.push(part);
    }

    pub fn resolve(&self, data: &DatasetData) -> String {
        let mut collect = Vec::with_capacity(self.parts.len());
        for part in &self.parts{
            collect.push(part.resolve(data));
        }
        collect.concat()
    }

    pub fn to_string(&self) -> String{
        let mut collect = Vec::with_capacity(self.parts.len());
        for part in &self.parts{
            collect.push(part.to_string());
        }
        collect.concat()
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiElementContentPart{
    Text(String),
    Data(Vec<String>),
}

impl UiElementContentPart{
    pub fn resolve(&self, mut data: &DatasetData) -> String{
        match self {
            UiElementContentPart::Text(t) => t.to_string(),
            UiElementContentPart::Data(property) => {
                for property in property{
                    data = data.get_property(property);
                }
                data.to_string()
            },
        }
    }

    pub fn to_string(&self) -> String{
        match self{
            UiElementContentPart::Text(s) => s.to_string(),
            UiElementContentPart::Data(property) => {
                let mut collect = vec!["<".to_string()];
                collect.push(property.join("."));
                collect.push(">".to_string());
                collect.concat()
            },
        }
    }
}

use std::vec;

use serde::{Deserialize, Serialize};

use crate::message::DatasetData;

/// A UiElementContent is the actual content that a UiElement renders.
/// It is a sequence of either text or references to data within a dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiElementContent{
    parts: Vec<UiElementContentPart>
}


impl UiElementContent{
    /// Create a new, empty UiElementContent
    pub fn new() -> Self{
        Self{
            parts: Vec::new()
        }
    }

    /// Create a new UiElementContent with a single text element populated by
    /// the provided String.
    pub fn new_text(text: String) -> Self{
        let parts = vec![UiElementContentPart::Text(text)];
        Self {
            parts
        }
    }

    /// Create a new UiElementContent with a single data element referring to
    /// the property named by the provided String.
    pub fn new_data(property: String) -> Self{
        let parts = vec![UiElementContentPart::Data(vec![property])];
        Self {
            parts
        }
    }

    /// Add a new [UiElementContentPart] to the end of the sequence.
    pub fn add_part(&mut self, part: UiElementContentPart){
        self.parts.push(part);
    }

    /// Return a String of the content, with the data references resolved to
    /// the data in the provided [DatasetData].
    pub fn resolve(&self, data: &DatasetData) -> String {
        let mut collect = Vec::with_capacity(self.parts.len());
        for part in &self.parts{
            collect.push(part.resolve(data));
        }
        collect.concat()
    }

    /// Return a String of the content, with the data references replaced with
    /// a textual representation of the reference. E.g. <name>
    pub fn to_string(&self) -> String{
        let mut collect = Vec::with_capacity(self.parts.len());
        for part in &self.parts{
            collect.push(part.to_string());
        }
        collect.concat()
    }
}


/// A UiElementContentPart represents part of the [UiElementContent].
/// It is usually in sequence with other parts to format text and data together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiElementContentPart{
    /// The UiElementContentPart is a String
    Text(String),
    /// The UiElementContentPart is a reference to some data in a dataset that
    /// must be resolved before it can be rendered.
    Data(Vec<String>),
}

impl UiElementContentPart{
    /// Return a String of the content part. If it is data, the reference will
    /// be resolved with the data in the provided [DatasetData].
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

    /// Return a String of the content, If it is data, the reference will be
    /// replaced with a textual representation of the reference. E.g. <name>
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

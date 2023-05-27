use spider_link::{
    message::{AbsoluteDatasetPath, DatasetData, UiInput, UiMessage},
    Relation,
};

use crate::processor::message::ProcessorMessage;

pub enum UiProcessorMessage {
    RemoteMessage(Relation, UiMessage),
    DatasetUpdate(AbsoluteDatasetPath, Vec<DatasetData>),
    SetSetting {
        header: String,
        title: String,
        inputs: Vec<(String, String)>,
        cb: fn(u32, &String, UiInput)->Option<ProcessorMessage>
    },
    RemoveSetting {
        header: String,
        title: String,
    },
    Upkeep,
}

impl std::fmt::Debug for UiProcessorMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RemoteMessage(arg0, arg1) => f
                .debug_tuple("RemoteMessage")
                .field(arg0)
                .field(arg1)
                .finish(),
            Self::DatasetUpdate(path, dataset) => f
                .debug_struct("DatasetUpdate")
                .field("path", path)
                .field("dataset", dataset)
                .finish(),
            Self::SetSetting {
                header,
                title,
                inputs,
                cb,
            } => f
                .debug_struct("SetSetting")
                .field("header", header)
                .field("title", title)
                .field("inputs", inputs)
                .field("cb", &"<redacted impl>")
                .finish(),
            Self::RemoveSetting {
                header,
                title
            } => f
                .debug_struct("SetSetting")
                .field("header", header)
                .field("title", title)
                .finish(),
            Self::Upkeep => write!(f, "Upkeep"),
        }
    }
}

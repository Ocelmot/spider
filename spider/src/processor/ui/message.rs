use spider_link::{
    message::{AbsoluteDatasetPath, DatasetData, UiInput, UiMessage},
    Relation,
};

use crate::processor::{dataset, sender::ProcessorSender};

pub enum UiProcessorMessage {
    RemoteMessage(Relation, UiMessage),
    DatasetUpdate(AbsoluteDatasetPath, Vec<DatasetData>),
    SetSetting {
        category: SettingCategory,
        name: String,
        setting_type: SettingType,
        callback: fn(&mut ProcessorSender, UiInput),
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
                category,
                name,
                setting_type,
                callback,
            } => f
                .debug_struct("SetSetting")
                .field("category", category)
                .field("name", name)
                .field("setting_type", setting_type)
                .field("callback", &"<redacted impl>")
                .finish(),
            Self::Upkeep => write!(f, "Upkeep"),
        }
    }
}

#[derive(Debug)]
pub enum SettingCategory {
    Test,
}

#[derive(Debug)]
pub enum SettingType {
    Button,
    Text,
}

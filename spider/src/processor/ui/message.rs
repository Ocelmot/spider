use spider_link::{
    message::{AbsoluteDatasetPath, DatasetData, UiInput, UiMessage},
    Relation,
};

use crate::processor::message::ProcessorMessage;

pub enum UiProcessorMessage {
    RemoteMessage(Relation, UiMessage),
    DatasetUpdate(AbsoluteDatasetPath, Vec<DatasetData>),
    SetSettingHeader {
        header: String,
    },
    SetSetting {
        header: String,
        title: String,
        inputs: Vec<(String, String)>,
        // cb: fn(u32, &String, UiInput, &mut String)->Option<ProcessorMessage>,
        cb: fn(&mut SettingEvent) -> Option<ProcessorMessage>,
        // cb: Box<dyn FnMut(u32, &String, UiInput)->Option<ProcessorMessage>>
        data: String,
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
            Self::SetSettingHeader { header } => f
                .debug_struct("SetSettingHeader")
                .field("header", header)
                .finish(),
            Self::SetSetting {
                header,
                title,
                inputs,
                cb: _,
                data,
            } => f
                .debug_struct("SetSetting")
                .field("header", header)
                .field("title", title)
                .field("inputs", inputs)
                .field("cb", &"<redacted impl>")
                .field("data", data)
                .finish(),
            Self::RemoveSetting { header, title } => f
                .debug_struct("SetSetting")
                .field("header", header)
                .field("title", title)
                .finish(),
            Self::Upkeep => write!(f, "Upkeep"),
        }
    }
}

pub struct SettingEvent<'a> {
    /// The [Relation] of the UI peripheral that sent this message.
    rel: Relation,
    /// The index into the array of inputs. Range: [0-9]
    index: u32,
    /// The title of the setting entry. This is the first item in the row.
    title: &'a String,
    /// The UnInput sent from the user
    input: UiInput,
    /// A reference to the data provided when the setting was assigned.
    data: &'a mut String,
}

impl<'a> SettingEvent<'a> {
    pub fn new(rel: Relation, index: u32, title: &'a String, input: UiInput, data: &'a mut String) -> Self {
        Self {
            rel,
            index,
            title,
            input,
            data,
        }
    }

    pub fn rel(&self) -> &Relation {
        &self.rel
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn title(&self) -> &'a String {
        self.title
    }

    pub fn input(&self) -> &UiInput {
        &self.input
    }

    pub fn data(&'a self) -> &'a String {
        &self.data
    }
    
    pub fn data_mut(&'a mut self) -> &'a mut String {
        self.data
    }
}

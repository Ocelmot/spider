use serde::{Deserialize, Serialize};

use crate::SpiderId2048;

use super::UiElement;

mod path;
pub use path::UiPath;

mod page_list;
pub use page_list::UiPageList;

mod manager;
pub use manager::UiPageManager;

/// A UiPage represents a page in the Ui that a peripheral has registered to
/// display its state and accept inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPage {
    id: SpiderId2048,
    name: String,

    root: UiElement,
}

impl UiPage {
    /// Create a new UiPage with the given id and a default root element.
    pub fn new<S>(id: SpiderId2048, name: S) -> Self
    where
        S: Into<String>,
    {
        Self {
            id,
            name: name.into(),
            root: UiElement::from_string("<new page>"),
        }
    }

    /// Get the name of the UiPage
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the [SpiderId2048] of the UiPage
    pub fn id(&self) -> &SpiderId2048 {
        &self.id
    }
    /// Assign a new [SpiderId2048] to this UiPage
    pub fn set_id(&mut self, id: SpiderId2048) {
        self.id = id;
    }

    /// Get the root [UiElement] of this UiPage
    pub fn root(&self) -> &UiElement {
        &self.root
    }
}

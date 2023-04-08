

use serde::{Deserialize, Serialize};

use crate::SpiderId2048;

use super::UiElement;

mod path;
pub use path::UiPath;

mod page_list;
pub use page_list::UiPageList;

mod manager;
pub use manager::UiPageManager;


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPage {
    id: SpiderId2048,
    name: String,
	
	root: UiElement,

}

impl UiPage {
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

    pub fn name(&self) -> &str{
        &self.name
    }

    pub fn id(&self) -> &SpiderId2048{
        &self.id
    }
    pub fn set_id(&mut self, id: SpiderId2048){
        self.id = id;
    }

    pub fn root(&self) -> &UiElement {
        &self.root
    }
}

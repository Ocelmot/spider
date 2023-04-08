use serde::{Deserialize, Serialize};

use crate::SpiderId2048;

mod page;
pub use page::{
	UiPage,
    UiPageManager,
	UiPageList,
    UiPath,
};

mod element;
pub use element::{
	UiElement,
	UiElementKind,
    UiElementUpdate,
};

mod input;
pub use input::UiInput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiMessage {
    // Base <---> UI Peripheral
    Subscribe,
    GetPages,
    Pages(Vec<UiPage>),
    GetPage(SpiderId2048),
    Page(UiPage),
    UpdateElementsFor(SpiderId2048, Vec<UiElementUpdate>),
    InputFor(SpiderId2048, String, UiInput),

    //Peripheral page <---> Base
    SetPage(UiPage),
    ClearPage,
    UpdateElements(Vec<UiElementUpdate>),
    Input(String, UiInput),
}

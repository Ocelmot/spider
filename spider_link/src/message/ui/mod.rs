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
    UiElementChange,
    UiElementContent,
    UiElementContentPart,
    
    UiChildOperations,

    UpdateSummary,
};

mod input;
pub use input::UiInput;

use super::{AbsoluteDatasetPath, DatasetData};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiMessage {
    // Base <---> UI Peripheral
    Subscribe,
    Pages(Vec<UiPage>),
    GetPage(SpiderId2048),
    Page(UiPage),
    UpdateElementsFor(SpiderId2048, Vec<UiElementUpdate>),
    Dataset(AbsoluteDatasetPath, Vec<DatasetData>),
    InputFor(SpiderId2048, String, Vec<usize>, UiInput),

    //Peripheral page <---> Base
    SetPage(UiPage),
    ClearPage,
    UpdateElements(Vec<UiElementUpdate>),
    Input(String, Vec<usize>, UiInput),
}

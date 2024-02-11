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

/// A UiMessage is used to synchronize the state of [UiPage]s between the base,
/// the UI peripheral, and the peripheral that is controlling the page.
/// UiMessages are divided into two categories: Definition and update of the
/// page, and user inputs from the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiMessage {
    // Base <---> UI Peripheral
    /// Subscribe as a UI Peripheral. Peripherals that subscribe will be sent
    /// the current set of [UiPage]s and thier state. They will also be sent
    /// any updates to the pages and thier state.
    /// This also subscribes this peripheral to any datasets that any [UiPage]s
    /// depend on.
    Subscribe,
    /// Transfer the current set of [UiPage]s
    Pages(Vec<UiPage>),
    /// Request the current state of the [UiPage] for a particular peripheral.
    GetPage(SpiderId2048),
    /// A singular [UiPage]. A Response to [UiMessage::GetPage].
    Page(UiPage),
    /// A Vec<[UiElementUpdate]> to be applied to the [UiPage] identified by
    /// the [SpiderId2048]
    UpdateElementsFor(SpiderId2048, Vec<UiElementUpdate>),
    /// An updated dataset that a [UiPage] depends on.
    Dataset(AbsoluteDatasetPath, Vec<DatasetData>),
    /// The user has provided input for a [UiPage] for some peripheral.
    InputFor(SpiderId2048, String, Vec<usize>, UiInput),

    //Peripheral page <---> Base
    /// This peripheral is setting its [UiPage]
    SetPage(UiPage),
    /// This peripheral is removing its [UiPage]
    ClearPage,
    /// This peripheral is updating a portion of its [UiPage]
    UpdateElements(Vec<UiElementUpdate>),
    /// The base is providing this peripheral with user input from its [UiPage].
    Input(String, Vec<usize>, UiInput),
}

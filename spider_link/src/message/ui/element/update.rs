use serde::{Deserialize, Serialize};

use crate::message::ui::page::UiPath;

use super::{change::UiChildOperations, UiElement};

#[derive(Debug, Clone, Serialize, Deserialize)]

/// A UiElementUpdate represents the modification of a [UiElement] within a
/// page that can be used to synchronize changes between two pages.
/// 
/// A UiElementUpdate can include changes to a [UiElement], or a set of changes
/// to that element's children, or both.
pub struct UiElementUpdate {
    path: UiPath,
    element: Option<UiElement>,
    children: Option<Vec<UiChildOperations>>,
}

impl UiElementUpdate {
    /// Create a new UiElementUpdate representing the changes to the [UiElement]
    /// indicated by the given path.
    pub fn update_element(path: UiPath, mut element: UiElement) -> Self {
        // Dont transmit children. They will be added through other updates.
        element.children = None;
        let element = Some(element);
        Self {
            path,
            element,
            children: None,
        }
    }

    /// Create a new UiElementUpdate representing the changes to the set of
    /// children of the [UiElement] indicated by the given path.
    /// 
    /// This does not include a modification of the children themselves
    /// only thier order or existance.
    pub fn update_children(path: UiPath, children: Vec<UiChildOperations>) -> Self {
        Self {
            path,
            element: None,
            children: Some(children),
        }
    }

    /// Create a new UiElementUpdate representing the changes to the set of
    /// children of the [UiElement] indicated by the given path, as well as
    /// changes to the UiElement itself.
    /// 
    /// This does not include a modification of the children themselves
    /// only thier order or existance.
    pub fn update_element_children(
        path: UiPath,
        mut element: UiElement,
        children: Vec<UiChildOperations>,
    ) -> Self {
        // Dont transmit children. They will be added through other updates.
        element.children = None;
        let element = Some(element);
        Self {
            path,
            element,
            children: Some(children),
        }
    }

    /// Get a reference to the [UiPath] this UiElementUpdate modifies
    pub fn path(&self) -> &UiPath {
        &self.path
    }

    /// Get a reference to the [UiElement] the UiElementUpdate describes.
    pub fn element(&self) -> &Option<UiElement> {
        &self.element
    }

    /// Get a reference to the changes in children
    /// the UiElementUpdate describes.
    pub fn children(&self) -> &Option<Vec<UiChildOperations>> {
        &self.children
    }

    /// Take the [UiElement] the UiElementUpdate describes,
    /// leaving None in its place.
    pub fn take_element(&mut self) -> Option<UiElement> {
        self.element.take()
    }

    /// Take the changes to the children the UiElementUpdate describes,
    /// leaving None in its place.
    pub fn take_children(&mut self) -> Option<Vec<UiChildOperations>> {
        self.children.take()
    }
}

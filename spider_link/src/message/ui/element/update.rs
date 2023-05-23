use serde::{Deserialize, Serialize};

use crate::message::{ui::page::UiPath};

use super::{change::UiChildOperations, UiElement};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiElementUpdate {
    path: UiPath,
    element: Option<UiElement>,
    children: Option<Vec<UiChildOperations>>,
}

impl UiElementUpdate {
    pub fn update_element(path: UiPath, mut element: UiElement) -> Self {
        element.children = None; // dont transmit children.
                                 //They will be added through other updates
        let element = Some(element);
        Self {
            path,
            element,
            children: None,
        }
    }

    pub fn update_children(path: UiPath, children: Vec<UiChildOperations>) -> Self {
        Self {
            path,
            element: None,
            children: Some(children),
        }
    }

    pub fn update_element_children(
        path: UiPath,
        mut element: UiElement,
        children: Vec<UiChildOperations>,
    ) -> Self {
        element.children = None; // dont transmit children.
                                 //They will be added through other updates
        let element = Some(element);
        Self {
            path,
            element,
            children: Some(children),
        }
    }

    pub fn path(&self) -> &UiPath {
        &self.path
    }

    pub fn element(&self) -> &Option<UiElement> {
        &self.element
    }

    pub fn children(&self) -> &Option<Vec<UiChildOperations>> {
        &self.children
    }

    pub fn take_element(&mut self) -> Option<UiElement> {
        self.element.take()
    }

    pub fn take_children(&mut self) -> Option<Vec<UiChildOperations>> {
        self.children.take()
    }
}

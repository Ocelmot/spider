
use std::slice::Iter;

use serde::{Deserialize, Serialize};

/// A UiPath refers to a UiElement within a UiPage
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UiPath{
    pub(super) path: Vec<usize>
}

impl UiPath {
    /// Create a new UiPath refering to the root of the UiPage
    pub fn root() -> Self{
        Self {
            path: Vec::new()
        }
    }

    /// Returns an iterator over the indices of the children of the UiElements
    /// in the UiPage
    pub fn iter(&self) -> Iter<usize> {
        self.path.iter()
    }

    /// Modifies the path to refer to the parent of the current element.
    /// Returns true if the operation was successful, and false when the path
    /// was refering to the root element and could not move to the parent.
    pub fn parent_of(&mut self) -> bool {
        match self.path.pop(){
            Some(_) => true,
            None => false,
        }
    }

    /// Append another UiPath to this one.
    pub fn append(&mut self, mut other: UiPath){
        self.path.append(&mut other.path);
    }

    /// Refer to a child of the current element, indicated by the child index.
    pub fn append_child(&mut self, child: usize){
        self.path.push(child);
    }
}

impl PartialOrd for UiPath {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let comparison = match self.path.len().cmp(&other.path.len()) {
            std::cmp::Ordering::Less => std::cmp::Ordering::Less,
            std::cmp::Ordering::Equal => {
                match self.path.last() {
                    Some(self_last_index) => {
                        let other_last_index = other.path.last().unwrap();
                        self_last_index.cmp(other_last_index)
                    },
                    None => std::cmp::Ordering::Equal,
                }
            },
            std::cmp::Ordering::Greater => std::cmp::Ordering::Greater,
        };
        Some(comparison)
    }
}

impl Ord for UiPath {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}
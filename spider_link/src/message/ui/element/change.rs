use std::{
    collections::{btree_map::IntoIter, BTreeMap},
    mem,
};

use serde::{Deserialize, Serialize};

use crate::message::ui::page::UiPath;

use super::UiElement;

#[derive(Debug, Clone, Default)]
pub struct UiElementChange {
    this_changed: bool,
    children_accessed: bool,
    child_operations: Vec<UiChildOperations>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiChildOperations {
    Insert(usize, UiElement),
    Delete(usize),
    MoveTo { from: usize, to: usize },
}

impl UiElementChange {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn take(&mut self) -> Self {
        let mut new = Self::new();
        mem::swap(self, &mut new);
        new
    }

    pub fn update(&mut self, other: &UiElementChange) {
        self.this_changed |= other.this_changed;
        self.children_accessed |= other.children_accessed;
        self.child_operations
            .append(&mut other.child_operations.clone());
    }
    pub fn absorb(&mut self, mut other: UiElementChange) {
        self.this_changed |= other.this_changed;
        self.children_accessed |= other.children_accessed;
        self.child_operations.append(&mut other.child_operations);
    }

    pub fn changed(&self) -> bool {
        self.this_changed
    }
    pub fn set_changed(&mut self) {
        self.this_changed = true;
    }
    pub fn clear_changed(&mut self) {
        self.this_changed = false;
    }

    pub fn children_accessed(&self) -> bool {
        self.children_accessed
    }
    pub fn set_children_accessed(&mut self) {
        self.children_accessed = true;
    }
    pub fn clear_children_accessed(&mut self) {
        self.children_accessed = false;
    }

    pub fn add_operation(&mut self, op: UiChildOperations) {
        self.child_operations.push(op);
    }
    pub fn take_operations(&mut self) -> Vec<UiChildOperations> {
        let mut taken = Vec::new();
        mem::swap(&mut self.child_operations, &mut taken);
        taken
    }
}

#[derive(Debug, Clone, Default)]
pub struct UiElementChangeSet {
    changes: BTreeMap<UiPath, UiElementChange>,
}

impl UiElementChangeSet {
    pub fn new() -> Self {
        let mut changes = BTreeMap::new();
        changes.insert(UiPath::root(), UiElementChange::new());
        Self { changes }
    }

    pub fn root(&mut self) -> &mut UiElementChange {
        if !self.changes.contains_key(&UiPath::root()) {
            self.changes.insert(UiPath::root(), UiElementChange::new());
        }
        self.changes.get_mut(&UiPath::root()).unwrap()
    }

    pub fn clear(&mut self){
        self.changes.clear();
    }

    /// absorb changes from other tree at path location
    pub fn absorb_at(&mut self, path: &UiPath, other: &mut UiElementChangeSet) {
        let mut other_changes = BTreeMap::new();
        mem::swap(&mut other_changes, &mut other.changes);
        for (other_path, other_element) in other_changes.into_iter() {
            let mut new_path = path.clone();
            new_path.append(other_path);

            match self.changes.get_mut(&new_path) {
                Some(existing_element) => {
                    existing_element.absorb(other_element);
                }
                None => {
                    self.changes.insert(new_path, other_element);
                }
            }
        }
    }

    pub fn absorb_at_index(&mut self, index: usize, other: &mut UiElementChangeSet) {
        let mut path = UiPath::root();
        path.append_child(index);
        self.absorb_at(&path, other);
    }

    pub fn take_changes_iter(&mut self) -> IntoIter<UiPath, UiElementChange> {
        let mut new_btree = BTreeMap::new();
        mem::swap(&mut new_btree, &mut self.changes);
        new_btree.into_iter()
    }
}

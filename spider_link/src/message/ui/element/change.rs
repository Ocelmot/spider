use std::{
    collections::{btree_map::IntoIter, BTreeMap},
    mem,
};

use serde::{Deserialize, Serialize};

use crate::message::ui::page::UiPath;

use super::UiElement;

/// A UiElementChange tracks if a [UiElement] has changed, if its children were
/// accessed, and which operations were performed on its children. This is done
/// in order to track changes to the elements.
#[derive(Debug, Clone, Default)]
pub struct UiElementChange {
    this_changed: bool,
    children_accessed: bool,
    child_operations: Vec<UiChildOperations>,
}

/// A change to the set of children that a [UiElement] could make.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiChildOperations {
    /// Insert a [UiElement] as a child at the indicated position.
    Insert(usize, UiElement),
    /// Delete the [UiElement] at the indicated position.
    Delete(usize),
    /// Move the [UiElement] from the one indicated position to the other.
    MoveTo {
        /// Move the [UiElement] from this position
        from: usize,
        /// Move the [UiElement] to this position
        to: usize
    },
}

impl UiElementChange {
    /// Create a new, default [UiElementChange].
    pub fn new() -> Self {
        Self::default()
    }
    /// Take the UiElementChange refered to by mutable reference, leaving a
    /// new, empty one in its place.
    pub fn take(&mut self) -> Self {
        let mut new = Self::new();
        mem::swap(self, &mut new);
        new
    }

    /// Update this UiElementChange with another by reference, cloning its
    /// child operations. 
    pub fn update(&mut self, other: &UiElementChange) {
        self.this_changed |= other.this_changed;
        self.children_accessed |= other.children_accessed;
        self.child_operations
            .append(&mut other.child_operations.clone());
    }
    /// Update this UiElementChange with another by ownership, without cloning
    /// its child operations.
    pub fn absorb(&mut self, mut other: UiElementChange) {
        self.this_changed |= other.this_changed;
        self.children_accessed |= other.children_accessed;
        self.child_operations.append(&mut other.child_operations);
    }

    /// Has this UiElementChange been changed?
    pub fn changed(&self) -> bool {
        self.this_changed
    }
    /// Set the changed flag in this UiElementChange to indicate that a change
    /// has occurred
    pub fn set_changed(&mut self) {
        self.this_changed = true;
    }
    /// Set the changed flag in this UiElementChange to indicate that a change
    /// has not occurred
    pub fn clear_changed(&mut self) {
        self.this_changed = false;
    }

    /// Has any child of this UiElementChange been accessed?
    pub fn children_accessed(&self) -> bool {
        self.children_accessed
    }
    /// Set the children accessed flag in this UiElementChange to indicate that
    /// a child was accessed.
    pub fn set_children_accessed(&mut self) {
        self.children_accessed = true;
    }
    /// Set the children accessed flag in this UiElementChange to indicate that
    /// a child was not accessed.
    pub fn clear_children_accessed(&mut self) {
        self.children_accessed = false;
    }

    /// Append a [UiChildOperations] to this UiElementChange's set of child
    /// operations.
    pub fn add_operation(&mut self, op: UiChildOperations) {
        self.child_operations.push(op);
    }
    /// Take the Vec<[UiChildOperations]> from this UiElementChange,
    /// leaving an empty Vec in its place.
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

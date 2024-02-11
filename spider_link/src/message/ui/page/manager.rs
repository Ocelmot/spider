use std::{
    collections::{BTreeSet, HashMap},
    mem,
};

use crate::{
    message::{
        ui::element::{UiElementChangeSet, UiElementRef, UiElementUpdate, UpdateSummary},
        UiElement,
    },
    SpiderId2048,
};

use super::{UiPage, UiPath};

/// A UiPageManager wraps a [UiPage]. The wrapped UiPage is accessed via the
/// UiPageManager, and all changes are captured. The captured changes can
/// then be gathered into a Vec<[UiElementUpdate]>.
/// 
/// These updates can then be applied to another UiPage through another
/// UiPageManager.
pub struct UiPageManager {
    page: UiPage,
    ids: HashMap<String, UiPath>,
    changed_nodes: BTreeSet<UiPath>,
    change_set: UiElementChangeSet,
}

impl UiPageManager {
    /// Create a new UiPageManager that wraps a new [UiPage]. The UiPage will
    /// be created with the provided id and name.
    pub fn new<S>(id: SpiderId2048, name: S) -> Self
    where
        S: Into<String>,
    {
        Self {
            page: UiPage::new(id, name),
            ids: HashMap::new(),
            changed_nodes: BTreeSet::new(),
            change_set: UiElementChangeSet::new(),
        }
    }

    /// Create a new UiPageManager that wraps the provided [UiPage].
    pub fn from_page(page: UiPage) -> Self {
        let mut ret = Self {
            page,
            ids: HashMap::new(),
            changed_nodes: BTreeSet::new(),
            change_set: UiElementChangeSet::new(),
        };
        ret.recalculate_ids();
        ret
    }

    /// Get a reference to the wrapped [UiPage].
    pub fn get_page(&self) -> &UiPage {
        &self.page
    }

    /// Replace the wrapped [UiPage] with a newly provided one.
    /// The old page is returned.
    pub fn set_page(&mut self, mut page: UiPage) -> UiPage {
        mem::swap(&mut self.page, &mut page);
        self.changed_nodes.clear();
        self.recalculate_ids();
        page
    }

    fn recalculate_ids(&mut self) {
        Self::recalculate_ids_node(&mut self.ids, &self.page.root, UiPath::root());
    }
    fn recalculate_ids_node(ids: &mut HashMap<String, UiPath>, node: &UiElement, path: UiPath) {
        if let Some(id) = node.id() {
            ids.insert(id.clone(), path.clone());
        }
        for (index, child) in node.children().enumerate() {
            let mut child_path = path.clone();
            child_path.append_child(index);
            Self::recalculate_ids_node(ids, child, child_path);
        }
    }

    /// Get a reference to a [UiElement] in the wrapped [UiPage] using the
    /// provided [UiPath] to determine which element.
    pub fn get_element(&self, path: &UiPath) -> Option<&UiElement> {
        let mut cursor = &self.page.root;
        for child_index in path.iter() {
            cursor = match cursor.get_child(*child_index) {
                Some(child) => child,
                None => return None,
            }
        }
        Some(cursor)
    }

    /// Get a [UiElementRef] that refers to an element in the wrapped [UiPage]
    /// using the provided [UiPath] to determine which element.
    /// Making changes to a [UiElement] through a [UiElementRef] allows
    /// changes to be captured by the UiPageManager.
    pub fn get_element_mut(&mut self, path: &UiPath) -> Option<UiElementRef> {
        let mut cursor = &mut self.page.root;
        for child_index in path.iter() {
            cursor = match cursor.get_child_mut(*child_index) {
                Some(child) => child,
                None => return None,
            }
        }
        self.changed_nodes.insert(path.clone());
        Some(UiElementRef::with_element_ref(cursor))
    }

    /// Get a mutable reference to a [UiElement] in the wrapped [UiPage] using
    /// the provided [UiPath] to determine which element.
    /// The UiPageManager will not be able to track elements
    /// referenced in this way.
    pub fn get_element_raw(&mut self, path: &UiPath) -> Option<&mut UiElement> {
        let mut cursor = &mut self.page.root;
        for child_index in path.iter() {
            cursor = match cursor.get_child_mut(*child_index) {
                Some(child) => child,
                None => return None,
            }
        }
        Some(cursor)
    }

    /// Get a [UiPath] to an element in the [UiPage] determined by that
    /// element's id.
    pub fn get_path(&self, id: &str) -> Option<&UiPath> {
        self.ids.get(id)
    }

    /// Get a reference to a [UiElement] in the [UiPage] by its id.
    pub fn get_by_id(&self, id: &str) -> Option<&UiElement> {
        match self.ids.get(id) {
            Some(path) => self.get_element(path),
            None => None,
        }
    }

    /// Get a [UiElementRef] that refers to an element in the [UiPage]
    /// determined by its id.
    pub fn get_by_id_mut(&mut self, id: &str) -> Option<UiElementRef> {
        let path = match self.ids.get(id) {
            Some(path) => path.clone(),
            None => return None,
        };
        self.get_element_mut(&path)
    }
    /// Get a mutable reference to a [UiElement] in the [UiPage]
    /// determined by its id.
    /// Changes made to the element will not be tracked by the UiPageManager.
    pub fn get_by_id_raw(&mut self, id: &str) -> Option<&mut UiElement> {
        let path = match self.ids.get(id) {
            Some(path) => path.clone(),
            None => return None,
        };
        self.get_element_raw(&path)
    }

    fn consolidate_changes(&mut self) {
        for path in mem::take(&mut self.changed_nodes) {
            if let Some(node) = self.get_element_raw(&path) {
                let mut changes = node.take_changes();
                self.change_set.absorb_at(&path, &mut changes);
            }
        }
    }

    /// Return all changes to elements in the [UiPage] as a
    /// Vec<[UiElementUpdate]>.
    /// These changes will not be included in future calls.
    pub fn get_changes(&mut self) -> Vec<UiElementUpdate> {
        self.consolidate_changes();
        let mut ret = Vec::new();
        for (path, mut change) in self.change_set.take_changes_iter() {
            // get element
            let element = match self.get_element(&path) {
                Some(e) => e,
                None => continue, // cant find this element in the tree, skip
            };

            // get children
            let children_part = change.take_operations();

            if change.changed() && !children_part.is_empty() {
                // update both
                let update =
                    UiElementUpdate::update_element_children(path, element.clone(), children_part);
                ret.push(update);
            } else {
                // update only one (or zero)
                if change.changed() {
                    // update element only
                    let update = UiElementUpdate::update_element(path, element.clone());
                    ret.push(update);
                } else if !children_part.is_empty() {
                    // update children only
                    let update = UiElementUpdate::update_children(path, children_part);
                    ret.push(update);
                }
            }
        }
        self.recalculate_ids(); // Could change this to only update ids that have changed per the new updates
        ret
    }

    /// Apply a Vec<[UiElementUpdate]> to the [UiPage]. If manual changes were
    /// made to this UiPage, and the UiPage to which this is being synchronized
    /// differ, those changes are ignored by the update and could cause 
    /// errors to occur.
    pub fn apply_changes(&mut self, changes: Vec<UiElementUpdate>) -> UpdateSummary {
        let mut ret = UpdateSummary::new();

        self.consolidate_changes();
        self.change_set.clear();

        for change in changes {
            let element = self.get_element_raw(change.path());
            match element {
                Some(element) => {
                    element.apply_update(change, &mut ret);
                }
                None => {
                    // could not find element to update, need to resync
                    return ret;
                }
            }
        }
        self.recalculate_ids(); // Could change this to only update ids that have changed per the new updates
        return ret;
    }
}

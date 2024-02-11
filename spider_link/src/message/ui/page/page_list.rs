use std::collections::BTreeMap;

use crate::SpiderId2048;

use super::{UiPage, UiPageManager};

/// A UiPageList holds a set of [UiPageManager]s in a particular order.
/// The [UiPageManager]s are stored in a tree, and the order is maintained
/// in a Vec.
pub struct UiPageList {
    order: Vec<SpiderId2048>,
    pages: BTreeMap<SpiderId2048, UiPageManager>,
    selected_page: usize,
}

impl UiPageList {
	/// Create a new, empty UiPageList
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            pages: BTreeMap::new(),
            selected_page: 0,
        }
    }

	/// Clear all pages out of the list
    pub fn clear(&mut self) {
        self.order.clear();
        self.pages.clear();
        self.selected_page = 0;
    }

	/// Add a batch of pages to the list. If a page exists, it is updated and
	/// remains at its position. If a page does not exist, it is appended to
	/// the end of the list.
    pub fn add_pages(&mut self, pages: Vec<UiPage>) {
        for page in pages {
            self.upsert_page(page);
        }
    }

	/// Update a [UiPageManager] in the list without changing its position.
	/// If the [UiPageManager] is not in the list, it is added to the end.
    pub fn upsert_page(&mut self, page: UiPage) -> Option<UiPage> {
        match self.pages.get_mut(&page.id) {
            Some(p) => {
                // page exists, replace it
                Some(p.set_page(page))
            }
            None => {
                // new page, add at end
                let key = page.id.clone();
                let page_manager = UiPageManager::from_page(page);
                self.pages.insert(key.clone(), page_manager);
                self.order.push(key);
                None
            }
        }
    }

	/// Get a [UiPageManager] from the list
    pub fn get_page(&self, id: &SpiderId2048) -> Option<&UiPageManager> {
        self.pages.get(&id)
    }

	/// Mutabily get a [UiPageManager] from the list
    pub fn get_page_mut(&mut self, id: &SpiderId2048) -> Option<&mut UiPageManager> {
        self.pages.get_mut(&id)
    }

	/// Get a Vec of [UiPage]s in this list
    pub fn get_page_vec(&self) -> Vec<&UiPage> {
        let mut list = Vec::new();
        for id in &self.order {
            let page = self.pages.get(id).unwrap();
            list.push(page.get_page());
        }
        list
    }

	/// Get a Vec of cloned [UiPage]s from this list
    pub fn clone_page_vec(&self) -> Vec<UiPage> {
        let mut list = Vec::new();
        for id in &self.order {
            let page = self.pages.get(id).unwrap();
            list.push(page.get_page().clone());
        }
        list
    }

	/// Get a Vec of [UiPageManager]s from this list
    pub fn get_mgr_vec(&self) -> Vec<&UiPageManager> {
        let mut list = Vec::new();
        for id in &self.order {
            let page = self.pages.get(id).unwrap();
            list.push(page);
        }
        list
    }

	/// Get the index of the selected page in this list
    pub fn selected_index(&self) -> usize {
        self.selected_page
    }

	/// Get a reference to the [UiPageManager] of the selected page in this list.
    pub fn selected_page(&self) -> Option<&UiPageManager> {
        let id = match self.order.get(self.selected_page) {
            Some(id) => id,
            None => return None,
        };
        self.pages.get(id)
    }

	/// Mutably et a reference to the [UiPageManager] of the selected page in
	/// this list.
    pub fn selected_page_mut(&mut self) -> Option<&mut UiPageManager> {
        let id = match self.order.get(self.selected_page) {
            Some(id) => id,
            None => return None,
        };
        self.pages.get_mut(id)
    }

	/// Select the page before the currently selected one.
    pub fn select_prev_page(&mut self) {
        if self.order.len() <= 1 {
            self.selected_page = 0;
        } else {
            if self.selected_page >= self.order.len() {
                self.selected_page = self.order.len() - 1
            }
            if self.selected_page != 0 {
                self.selected_page -= 1;
            }
        }
    }

	/// Select the page after the currently selected one.
    pub fn select_next_page(&mut self) {
        if self.order.len() <= 1 {
            self.selected_page = 0;
        } else {
            self.selected_page += 1;
            if self.selected_page >= self.order.len() {
                self.selected_page = self.order.len() - 1
            }
        }
    }
}

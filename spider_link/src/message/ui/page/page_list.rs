use std::collections::BTreeMap;

use crate::SpiderId2048;

use super::{UiPage, UiPageManager};












pub struct UiPageList{
	order: Vec<SpiderId2048>,
	pages: BTreeMap<SpiderId2048, UiPageManager>,
	selected_page: usize,
}

impl UiPageList{
	pub fn new() -> Self{

		Self {
			order: Vec::new(),
			pages: BTreeMap::new(),
			selected_page: 0,
		}
	}

	pub fn clear(&mut self){
		self.order.clear();
		self.pages.clear();
		self.selected_page = 0;
	}

	pub fn add_pages(&mut self, pages: Vec<UiPage>){
		for page in pages{
			self.upsert_page(page);
		}
	}

	pub fn upsert_page(&mut self, page: UiPage) -> Option<UiPage>{
		match self.pages.get_mut(&page.id) {
			Some(p) => { // page exists, replace it
				Some(p.set_page(page))
			},
			None => { // new page, add at end
				let key = page.id.clone();
				let page_manager = UiPageManager::from_page(page);
				self.pages.insert(key.clone(), page_manager);
				self.order.push(key);
				None
			},
		}
	}

	pub fn get_page_mut(&mut self, id: &SpiderId2048) -> Option<&mut UiPageManager>{
		self.pages.get_mut(&id)
	}

	pub fn get_page_vec(&self) -> Vec<&UiPage>{
		let mut list = Vec::new();
		for id in &self.order{
			let page = self.pages.get(id).unwrap();
			list.push(page.get_page());
		}
		list
	}

	pub fn clone_page_vec(&self) -> Vec<UiPage>{
		let mut list = Vec::new();
		for id in &self.order{
			let page = self.pages.get(id).unwrap();
			list.push(page.get_page().clone());
		}
		list
	}

	pub fn get_mgr_vec(&self) -> Vec<&UiPageManager>{
		let mut list = Vec::new();
		for id in &self.order{
			let page = self.pages.get(id).unwrap();
			list.push(page);
		}
		list
	}

	pub fn selected_index(&self) -> usize {
		self.selected_page
	}

	pub fn selected_page(&self) -> Option<&UiPageManager>{
		let id = match self.order.get(self.selected_page){
			Some(id) => id,
			None => return None,
		};
		self.pages.get(id)
	}

	pub fn selected_page_mut(&mut self) -> Option<&mut UiPageManager>{
		let id = match self.order.get(self.selected_page){
			Some(id) => id,
			None => return None,
		};
		self.pages.get_mut(id)
	}
	
	pub fn select_prev_page(&mut self){
		if self.order.len() <= 1{
			self.selected_page = 0;
		}else{
			if self.selected_page >= self.order.len(){
				self.selected_page = self.order.len()-1
			}
			if self.selected_page != 0{
				self.selected_page -= 1;
			}
		}
	}

	pub fn select_next_page(&mut self){
		if self.order.len() <= 1{
			self.selected_page = 0;
		}else{
			self.selected_page += 1;
			if self.selected_page >= self.order.len(){
				self.selected_page = self.order.len()-1
			}
		}
	}

}
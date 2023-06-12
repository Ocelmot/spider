use std::ops::{Deref, DerefMut};

use super::{UiElement, change::UiChildOperations};


pub struct UiElementRef<'a>{
    element: &'a mut UiElement,
}

impl<'a> UiElementRef<'a>{
    pub fn with_element_ref(element: &'a mut UiElement) -> Self {
        Self{
            element,
        }
    }

    pub fn get_child_mut(&'a mut self, index: usize) -> Option<UiElementRef<'a>> {
        match &mut self.element.children{
            Some(children) => {
                self.element.changes.root().set_children_accessed();
                match children.get_mut(index) {
                    Some(element) => Some(UiElementRef::with_element_ref(element)),
                    None => None,
                }
            },
            None => None,
        }
    }
    pub fn get_child_raw(&'a mut self, index: usize) -> Option<&'a mut UiElement> {
        self.element.get_child_mut(index)
    }

    pub fn children_mut(&mut self) -> std::slice::IterMut<UiElement> {
        match &mut self.element.children {
            Some(c) => {
                self.element.changes.root().set_children_accessed();
                c.iter_mut()
            },
            None => [].iter_mut(),
        }
    }

    pub fn children_raw(&mut self) -> std::slice::IterMut<UiElement> {
        self.element.children_mut()
    }

    pub fn insert_child(&mut self, index: usize, child: UiElement) {
        self.element.changes.root().add_operation(UiChildOperations::Insert(index, child.clone()));
        self.element.insert_child(index, child);
    }
    pub fn append_child(&mut self, child: UiElement) {
        let index = match &self.element.children {
            Some(children) => children.len(),
            None => 0,
        };
        self.element.changes.root().add_operation(UiChildOperations::Insert(index, child.clone()));
        self.element.insert_child(index, child);
    }

    pub fn delete_child(&mut self, index: usize) {
        self.element.changes.root().add_operation(UiChildOperations::Delete(index));
        self.element.delete_child(index);
    }

}

impl Deref for UiElementRef<'_>{
    type Target = UiElement;

    fn deref(&self) -> &Self::Target {
        self.element
    }
}

impl DerefMut for UiElementRef<'_>{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.element.changes.root().set_changed();
        self.element
    }
}

impl Drop for UiElementRef<'_>{
    fn drop(&mut self) {
        // hoist changes to self
        if self.element.changes.root().children_accessed(){
            match &mut self.element.children{
                Some(children) => {
                    for (index, child) in children.iter_mut().enumerate() {
                        self.element.changes.absorb_at_index(index, &mut child.changes);
                    }
                },
                None => {},
            }
        }
    }
}
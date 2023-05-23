use std::collections::HashMap;

use crate::message::AbsoluteDatasetPath;

use super::UiElement;





pub struct UpdateSummary{
    changed: bool,
    dataset_subscriptions: HashMap<AbsoluteDatasetPath, isize>,
}

impl UpdateSummary{
    pub fn new() -> Self{
        Self { 
            changed: false,
            dataset_subscriptions: HashMap::new(),
        }
    }

    // Getters and Setters
    pub fn changed(&self) -> bool {
        self.changed
    }

    pub fn dataset_subscriptions(&self) -> &HashMap<AbsoluteDatasetPath, isize>{
        &self.dataset_subscriptions
    }

    // Utility functions
    fn add_dataset(&mut self, path: &AbsoluteDatasetPath){
        match self.dataset_subscriptions.get_mut(&path) {
            Some(count) => {
                *count += 1;
            },
            None => {
                self.dataset_subscriptions.insert(path.clone(), 1);
            },
        }
    }
    fn remove_dataset(&mut self, path: &AbsoluteDatasetPath){
        match self.dataset_subscriptions.get_mut(&path) {
            Some(count) => {
                *count -= 1;
            },
            None => {
                self.dataset_subscriptions.insert(path.clone(), 1);
            },
        }
    }


    // Change registration
    pub fn element(&mut self, old: &UiElement, new: &UiElement){
        // Changed:
        self.changed |= old.kind != new.kind;
        self.changed |= old.id != new.id;
    
        self.changed |= old.content != new.content;
        self.changed |= old.alt_text != new.alt_text;

        // Dataset Changes:
        match &old.dataset{
            Some(old_path) => {
                match &new.dataset{
                    Some(new_path) => {
                        // if paths are the same -> no change
                        // if different -> remove old, add new
                        if old_path != new_path{
                            self.remove_dataset(old_path);
                            self.add_dataset(new_path);
                        }
                    },
                    None => {
                        // old, but not new -> remove old
                        self.remove_dataset(old_path);
                    },
                }
            },
            None => {
                match &new.dataset{
                    Some(new_path) => {
                        // no old, but new -> add new
                        self.add_dataset(new_path);
                    },
                    None => {
                        // old was none, and new is none -> no change
                    },
                }
            },
        }
    }

    pub fn add(&mut self, elem: &UiElement){
        // Changed: always changed
        self.changed = true;

        // Dataset Changes: walk elem, adding all
        if let Some(new_path) = &elem.dataset {
            self.add_dataset(&new_path);
        }
        for child in elem.children(){
            self.add(child);
        }
    }

    pub fn remove(&mut self, elem: &UiElement){
        // Changed: always changed
        self.changed = true;
        
        // Dataset Changes: walk elem, removing all
        if let Some(old_path) = &elem.dataset {
            self.remove_dataset(&old_path);
        }
        for child in elem.children(){
            self.remove(child);
        }
    }

    pub fn move_to(&mut self, _elem: &UiElement){
        // Changed: always changed
        self.changed = true;
        
        // Dataset Changes: no changes
    }

}

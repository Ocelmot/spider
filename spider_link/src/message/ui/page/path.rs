
use std::slice::Iter;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct UiPath{
    pub(super) path: Vec<usize>
}

impl UiPath {
    pub fn root() -> Self{
        Self {
            path: Vec::new()
        }
    }

    pub fn iter(&self) -> Iter<usize> {
        self.path.iter()
    }

    pub fn parent_of(&mut self) -> bool {
        match self.path.pop(){
            Some(_) => true,
            None => false,
        }
    }

    pub fn append(&mut self, mut other: UiPath){
        self.path.append(&mut other.path);
    }

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
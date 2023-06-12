use std::{collections::HashMap, iter::FusedIterator};

use serde::{Deserialize, Serialize};

mod content;
pub use content::{
    UiElementContent,
    UiElementContentPart,
};

mod change;
pub use change::{
    UiElementChange,
    UiChildOperations,
    UiElementChangeSet
};

mod update;
pub use update::UiElementUpdate;

mod update_summary;
pub use update_summary::UpdateSummary;

mod reference;
pub use reference::UiElementRef;

use crate::message::{AbsoluteDatasetPath, DatasetData};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiElement {
    kind: UiElementKind,
    id: Option<String>,
    selectable: bool,

    content: UiElementContent,
    alt_text: UiElementContent,

    dataset: Option<AbsoluteDatasetPath>,

    children: Option<Vec<UiElement>>,

    #[serde(skip)]
    changes: UiElementChangeSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiElementKind {
    None,

    // Layout
    Spacer,
    Columns,
    Rows,
    Grid(u8, u8),

    // Output
    Text,

    // Input
    TextEntry,
    Button,

    // Misc
    Variable(UiElementContentPart),
}

impl UiElementKind{
    pub fn selectable(&self) -> bool {
        match self{
            UiElementKind::None => false,
            UiElementKind::Spacer => false,
            UiElementKind::Columns => false,
            UiElementKind::Rows => false,
            UiElementKind::Grid(_, _) => false,
            UiElementKind::Text => false,
            UiElementKind::TextEntry => true,
            UiElementKind::Button => true,
            UiElementKind::Variable(_) =>false ,
        }
    }

    pub fn resolve(self, datum: &Option<&DatasetData>) -> UiElementKind{
        match datum{
            Some(datum) => {
                if let UiElementKind::Variable(content_part) = self.clone() {
                    let mut string = content_part.resolve(datum);
                    string = string.to_ascii_lowercase();
                    match string.as_str(){
                        "none" => UiElementKind::None,
                        "spacer" => UiElementKind::Spacer,
                        "columns" => UiElementKind::Columns,
                        "rows" => UiElementKind::Rows,

                        "text" => UiElementKind::Text,
                        "textentry" => UiElementKind::TextEntry,
                        "button" => UiElementKind::Button,
                        _ => self
                    }
                }else{
                    self
                }
            },
            None => self,
        }
    }
}

impl UiElement {
    pub fn new(kind: UiElementKind) -> Self {
        let selectable = kind.selectable();
        Self {
            kind,
            id: None,
            selectable,

            content: UiElementContent::new(),
            alt_text: UiElementContent::new(),

            dataset: None,

            children: Some(Vec::new()),

            changes: UiElementChangeSet::new(),
        }
    }

    pub fn from_string<S>(string: S) -> Self
    where
        S: Into<String>,
    {
        Self {
            kind: UiElementKind::Text,
            id: None,
            selectable: false,

            content: UiElementContent::new_text(string.into()),
            alt_text: UiElementContent::new(),

            dataset: None,

            children: Some(Vec::new()),

            changes: UiElementChangeSet::new(),
        }
    }

    // Accessors, etc.
    pub fn kind(&self) -> &UiElementKind{
        &self.kind
    }
    pub fn set_kind(&mut self, kind: UiElementKind){
        self.kind = kind;
    }

    pub fn id(&self) -> Option<&String>{
        self.id.as_ref()
    }
    pub fn set_id<S>(&mut self, id: S) 
    where
        S: Into<String>,
    {
        self.id = Some(id.into());
    }
    pub fn selectable(&self)->bool{
        self.selectable
    }
    pub fn set_selectable(&mut self, selectable: bool){
        self.selectable = selectable;
    }

    pub fn text(&self) -> String {
        self.content.to_string()
    }
    pub fn set_text<S>(&mut self, text: S)
    where
    S: Into<String>,
    {
        self.content = UiElementContent::new_text(text.into());
    }

    pub fn set_content(&mut self, content: UiElementContent){
        self.content = content;
    }

    pub fn dataset(&self) -> &Option<AbsoluteDatasetPath>{
        &self.dataset
    }
    pub fn set_dataset(&mut self, dataset: Option<AbsoluteDatasetPath>){
        self.dataset = dataset;
    }

    // Content operations
    pub fn render_content(&self, data: &DatasetData) -> String {
        self.content.resolve(data)
    }
    pub fn render_content_opt(&self, data: &Option<&DatasetData>) -> String {
        match data {
            Some(data) => {
                self.content.resolve(data)
            },
            None => {
                self.text()
            },
        }
    }

    // Child operations
    pub fn get_child<'a>(&'a self, index: usize) -> Option<&'a UiElement> {
        match &self.children{
            Some(children) => children.get(index),
            None => None,
        }
    }

    pub fn get_child_mut<'a>(&'a mut self, index: usize) -> Option<&'a mut UiElement> {
        match &mut self.children{
            Some(children) => {
                children.get_mut(index)
            },
            None => None,
        }
    }

    pub fn children(&self) -> std::slice::Iter<UiElement> {
        match &self.children{
            Some(children) => children.iter(),
            None => [].iter(),
        }
    }

    pub fn children_mut(&mut self) -> std::slice::IterMut<UiElement> {
        match &mut self.children {
            Some(c) => {
                // self.changes.root().set_children_accessed();
                c.iter_mut()
            },
            None => [].iter_mut(),
        }
    }

    pub fn children_dataset<'a>(&'a self, data: &'a Option<&DatasetData>, data_map: &'a HashMap<AbsoluteDatasetPath, Vec<DatasetData>>) -> UiElementDatasetIterator{
        UiElementDatasetIterator::new(&self, data, data_map)
    }

    pub fn insert_child(&mut self, index: usize, child: UiElement){
        match &mut self.children {
            Some(children) => {
                children.insert(index, child);
            },
            None => {
                let mut children = Vec::new();
                children.insert(index, child);
                self.children = Some(children);
            },
        }
    }
    pub fn append_child(&mut self, child: UiElement){
        let index = match &self.children{
            Some(children) => children.len(),
            None => 0usize,
        };
        self.insert_child(index, child);
    }
    pub fn delete_child(&mut self, index: usize){
        match &mut self.children {
            Some(children) => {
                children.remove(index);
            },
            None => {}, // no children to delete
        }
    }

    // Change management operations
    pub fn take_changes(&mut self) -> UiElementChangeSet{
        std::mem::take(&mut self.changes)
    }

    pub fn apply_update(&mut self, mut update: UiElementUpdate, summary: &mut UpdateSummary){
        // if node was changed, assign values to self
        if let Some(node_changes) = update.take_element(){
            summary.element(self, &node_changes);
            // assign from change to self
            self.kind = node_changes.kind;
            self.id = node_changes.id;
        
            self.content = node_changes.content;
            self.alt_text = node_changes.alt_text;
        }

        // apply changes to children
        if let Some(ref mut children) = self.children{
            if let Some(child_changes) = update.take_children(){

                for operation in child_changes{
                    match operation{
                        change::UiChildOperations::Insert(index, element) => {
                            summary.add(&element);
                            children.insert(index, element);
                        },
                        change::UiChildOperations::Delete(index) => {
                            let removed = children.remove(index);
                            summary.remove(&removed);
                        },
                        change::UiChildOperations::MoveTo { from, to } => {
                            let element = children.remove(from);
                            summary.move_to(&element);
                            children.insert(to, element);
                        },
                    }
                }
            }
        }
    }
}







pub struct UiElementDatasetIterator<'a>{
    // data references
    elem: &'a UiElement,
    data: &'a Option<&'a DatasetData>,
    dataset_map: &'a HashMap<AbsoluteDatasetPath, Vec<DatasetData>>,
    // front iterator: points to next items to return
    front_dataset: isize,
    front_child: isize,
    // back iterator
    back_dataset: isize,
    back_child: isize,
}

impl<'a> UiElementDatasetIterator<'a>{
    fn new(elem: &'a UiElement, data: &'a Option<&'a DatasetData>, dataset_map: &'a HashMap<AbsoluteDatasetPath, Vec<DatasetData>>) -> Self{
        let back_dataset = match &elem.dataset{
            Some(dataset) => {
                match dataset_map.get(&dataset){
                    Some(dataset) => (dataset.len() as isize) - 1,
                    None => 0,
                }
            },
            None => 0,
        };
        let back_child = match &elem.children{
            Some(children) => (children.len() as isize) - 1,
            None => 0,
        };
        Self{
            // data references
            elem,
            data,
            dataset_map,
            // front iterator
            front_dataset: 0, // index of next element to return
            front_child: 0,
            // back iterator
            back_dataset,
            back_child,
        }
    }

    fn is_done(&self) -> bool{
        if self.back_dataset < self.front_dataset{
            return true;
        }
        if self.back_dataset == self.front_dataset{
            if self.back_child < self.front_child{
                return true;
            }
        }
        return false;
    }

    fn advance_front(&mut self){
        self.front_child += 1;
        let child_len = self.elem.children.as_ref().map_or(0, |v|v.len() as isize);
        if self.front_child >= child_len {
            self.front_child = 0;
            self.front_dataset += 1;
        }
    }
    fn advance_back(&mut self){
        self.back_child -= 1;
        if self.back_child < 0 {
            let child_len = self.elem.children.as_ref().map_or(0, |v|v.len() as isize);
            self.back_child = child_len - 1;
            self.back_dataset -= 1;
        }
    }
}

impl<'a> Iterator for UiElementDatasetIterator<'a>{
    type Item = (Option<usize>, &'a UiElement, Option<&'a DatasetData>);

    fn next(&mut self) -> Option<Self::Item> {
        match &self.elem.children{
            Some(children) => {
                // if the elem has a dataset, iterate that
                
                match &self.elem.dataset{
                    Some(path) => {
                        match self.dataset_map.get(path) {
                            Some(dataset) => {
                                if self.is_done(){
                                    return None;
                                }

                                // get data
                                let index = self.front_dataset as usize;
                                let child = &children[self.front_child as usize];
                                let datum = &dataset[self.front_dataset as usize];
                                
                                // update indices
                                self.advance_front();

                                // return triplet
                                Some((Some(index), child, Some(datum)))
                            },
                            None => {
                                // there was no dataset.
                                // iterate through children once, and pass no data element
                                if self.is_done(){
                                    return None;
                                }

                                // get data
                                let index = self.front_dataset as usize;
                                let child = &children[self.front_child as usize];
                                let datum = &None;
                                
                                // update indices
                                self.advance_front();

                                // return triplet
                                Some((Some(index), child, datum.as_ref()))
                            },
                        }
                    },
                    None => {
                        // else, iterate normally
                        // test validity of indices
                        if self.is_done(){
                            return None;
                        }

                        // get data
                        let child = &children[self.front_child as usize];
                        let datum = self.data;
                        
                        // update indices
                        self.advance_front();

                        // return triplet
                        Some((None, child, *datum))
                    },
                }
            },
            None => None, // No children cause no elements
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.elem.dataset{
            Some(path) => {
                // there is a dataset path, add uniterated dataset sizes to total
                match self.dataset_map.get(&path) {
                    Some(_) => {
                        let l = (self.back_child - self.front_child + 1) as usize;
                        let d = self.back_dataset - self.front_dataset;
                        let t = l + (d as usize * self.elem.children().len());
                        (t, Some(t))
                    },
                    None => {
                        // there is no dataset, length is back - front
                        // +1 for len vs index discrepancy
                        let l = (self.back_child - self.front_child + 1) as usize;
                        (l, Some(l))        
                    },
                }
            },
            None => {
                // there is no dataset, length is back - front
                // +1 for len vs index discrepancy
                let l = (self.back_child - self.front_child + 1) as usize;
                (l, Some(l))
            },
        }
    }
}

impl<'a> DoubleEndedIterator for UiElementDatasetIterator<'a>{
    fn next_back(&mut self) -> Option<Self::Item> {
        match &self.elem.children{
            Some(children) => {
                // if the elem has a dataset, iterate that
                
                match &self.elem.dataset{
                    Some(path) => {
                        match self.dataset_map.get(path) {
                            Some(dataset) => {
                                if self.is_done(){
                                    return None;
                                }

                                // get data
                                let index = self.back_dataset as usize;
                                let child = &children[self.back_child as usize];
                                let datum = &dataset[self.back_dataset as usize];
                                
                                // update indices
                                self.advance_back();

                                // return triplet
                                Some((Some(index), child, Some(datum)))
                            },
                            None => {
                                // there was no dataset.
                                // iterate through children once, and pass no data element
                                if self.is_done(){
                                    return None;
                                }

                                // get data
                                let index = self.back_dataset as usize;
                                let child = &children[self.back_child as usize];
                                let datum = &None;
                                
                                // update indices
                                self.advance_back();

                                // return triplet
                                Some((Some(index), child, datum.as_ref()))
                            },
                        }
                    },
                    None => {
                        // else, iterate normally
                        // test validity of indices
                        if self.is_done(){
                            return None;
                        }

                        // get data
                        let child = &children[self.back_child as usize];
                        let datum = self.data;
                        
                        // update indices
                        self.advance_back();

                        // return triplet
                        Some((None, child, *datum))
                    },
                }
            },
            None => None, // No children cause no elements
        }
    }
}

impl<'a> ExactSizeIterator for UiElementDatasetIterator<'a>{}

impl<'a> FusedIterator for UiElementDatasetIterator<'a>{}

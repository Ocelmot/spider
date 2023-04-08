use serde::{Deserialize, Serialize};

mod change;
pub use change::UiElementChangeSet;

mod update;
pub use update::UiElementUpdate;

mod reference;
pub use reference::UiElementRef;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiElement {
    kind: UiElementKind,
    id: Option<String>,
    selectable: bool,

    text: String,
    alt_text: String,

    children: Option<Vec<UiElement>>,

    #[serde(skip)]
    changes: UiElementChangeSet,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum UiElementKind {
    // Layout
    Columns,
    Rows,
    Grid(u8, u8),

    // Output
    Text,

    // Input
    TextEntry,
    Button,
}

impl UiElementKind{
    pub fn selectable(&self) -> bool {
        match self{
            UiElementKind::Columns => false,
            UiElementKind::Rows => false,
            UiElementKind::Grid(_, _) => false,
            UiElementKind::Text => false,
            UiElementKind::TextEntry => true,
            UiElementKind::Button => true,
        }
    }
}

impl UiElement {
    pub fn new(kind: UiElementKind) -> Self {
        Self {
            kind,
            id: None,
            selectable: kind.selectable(),

            text: String::new(),
            alt_text: String::new(),

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

            text: string.into(),
            alt_text: String::new(),

            children: Some(Vec::new()),

            changes: UiElementChangeSet::new(),
        }
    }

    // Accessors, etc.
    pub fn kind(&self) -> UiElementKind{
        self.kind
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

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn set_text<S>(&mut self, text: S)
    where
    S: Into<String>,
    {
        self.text = text.into();
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

    // Change management operations
    pub fn take_changes(&mut self) -> UiElementChangeSet{
        std::mem::take(&mut self.changes)
    }

    pub fn apply_update(&mut self, mut update: UiElementUpdate){
        // if node was changed, assign values to self
        if let Some(node_changes) = update.take_element(){
            // assign from change to self
            self.kind = node_changes.kind;
            self.id = node_changes.id;
        
            self.text = node_changes.text;
            self.alt_text = node_changes.alt_text;
        }

        // apply changes to children
        if let Some(ref mut children) = self.children{
            if let Some(child_changes) = update.take_children(){

                for operation in child_changes{
                    match operation{
                        change::UiChildOperations::Insert(index, element) => {
                            children.insert(index, element);
                        },
                        change::UiChildOperations::Delete(index) => {
                            children.remove(index);
                        },
                        change::UiChildOperations::MoveTo { from, to } => {
                            let element = children.remove(from);
                            children.insert(to, element);
                        },
                    }
                }
            }
        }
    }
}

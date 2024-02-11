use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::SpiderId2048;



// ========== Absolute Path ==========

/// An AbsoluteDatasetScope describes if the [AbsoluteDatasetPath] refers to
/// a public dataset or to a peripheral's dataset, and if so, which peripheral.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AbsoluteDatasetScope{
    /// The [AbsoluteDatasetPath] refers to a dataset belonging to a peripheral
    /// with this id.
	Peripheral(SpiderId2048),
    /// The [AbsoluteDatasetPath] refers to a public dataset.
	Public,
}

/// An AbsoluteDatasetPath refers to some dataset stored within the base.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AbsoluteDatasetPath{
    /// Whether the dataset is public or belongs to some peripheral.
	scope: AbsoluteDatasetScope,
    /// A sequence of strings to refer to a dataset within the scope.
    name: Vec<String>,
}

impl AbsoluteDatasetPath{
    /// Create a new AbsoluteDatasetPath from the given sequence of strings
    /// and with [AbsoluteDatasetScope::Public] scope.
    pub fn new_public(name: Vec<String>) -> Self{
        Self { 
            scope: AbsoluteDatasetScope::Public,
            name
        }
    }

    /// Get the [AbsoluteDatasetScope] of this AbsoluteDatasetPath.
    pub fn scope(&self) -> &AbsoluteDatasetScope{
        &self.scope
    }
    /// Get the sequence of strings that refer to the dataset.
    pub fn parts(&self) -> &Vec<String>{
        &self.name
    }

    /// Convert this AbsoluteDatasetPath into a [DatasetPath], discarding the
    /// peripheral's id if present.
    pub fn specialize(self) -> DatasetPath{
        let scope = match self.scope{
            AbsoluteDatasetScope::Peripheral(_) => DatasetScope::Private,
            AbsoluteDatasetScope::Public => DatasetScope::Public,
        };
        DatasetPath {
            scope,
            name: self.name
        }
    }
}

// ========== Path ==========
/// A DatasetScope describes whether a [DatasetPath] is referring to a public or
/// private dataset. If it refers to a private dataset, the peripheral's id is
/// not included and must be inferred by context.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatasetScope{
    /// The dataset belongs to some peripheral.
	Private,
    /// The dataset is public.
	Public,
}

/// A DatasetPath describes if a dataset is public or private, and which dataset
/// within that scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DatasetPath{
	scope: DatasetScope,
    name: Vec<String>,
}

impl DatasetPath{
    /// Returns the [DatasetScope] for this DatasetPath.
    pub fn scope(&self) -> &DatasetScope{
        &self.scope
    }
    /// Returns a sequence of strings indicating which dataset with the scope.
    pub fn parts(&self) -> &Vec<String>{
        &self.name
    }

    /// Create a new DatasetPath with the given sequence of strings and
    /// [DatasetScope::Private] scope.
    pub fn new_private(name: Vec<String>) -> Self {
        Self {
            scope: DatasetScope::Private,
            name,
        }
    }

    /// Create a new DatasetPath with the given sequence of strings and
    /// [DatasetScope::Public] scope.
    pub fn new_public(name: Vec<String>) -> Self {
        Self {
            scope: DatasetScope::Public,
            name,
        }
    }

    /// Convert this DatasetPath into an [AbsoluteDatasetPath] belonging to a
    /// peripheral with the given id. If the scope is public,
    /// the id is not used.
    pub fn resolve(self, id: SpiderId2048) -> AbsoluteDatasetPath{
        let scope = match self.scope{
            DatasetScope::Private => AbsoluteDatasetScope::Peripheral(id),
            DatasetScope::Public => AbsoluteDatasetScope::Public,
        };
        AbsoluteDatasetPath {
            scope,
            name: self.name
        }
    }
}

// ========== Dataset Data and Message ===========
/// A DatasetData represents an entry in a dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatasetData{
    /// A null value
    Null,
    /// An unsigned byte
    Byte(u8),
    /// A 32bit integer
    Int(i32),
    /// A 32bit float
    Float(f32),
    /// A String
    String(String),

    /// An array of DatasetData
    Array(Vec<DatasetData>),
    /// A map from string keys to DatasetData entries
    Map(HashMap<String, DatasetData>),
}

/// A DatasetMessage represents operations on the datasets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatasetMessage{
    // Dataset Requests
    /// Request to be alerted to changes to the dataset described by
    /// [DatasetPath].
	Subscribe{
        /// The [DatasetPath] to the dataset to which to subscribe.
        path: DatasetPath
    },
    
    /// Append a [DatasetData] to the dataset described by the [DatasetPath]
    Append{
        /// The [DatasetPath] to the dataset to which to append.
        path: DatasetPath,
        /// The [DatasetData] to append to the dataset.
        data: DatasetData
    },

    /// Append a Vec<[DatasetData]> to the dataset described by
    /// the [DatasetPath].
    Extend{
        /// The [DatasetPath] to the dataset to extend.
        path: DatasetPath,
        /// A Vec<[DatasetData]> to extend the dataset.
        data: Vec<DatasetData>
    },

    /// Set the element at the given id to the given [DatasetData], in the
    /// dataset described by the [DatasetPath]. If the id refers to a position
    /// after the end of the dataset, the dataset will be padded with
    /// [DatasetData::Null].
    SetElement{
        /// The [DatasetPath] to the dataset to modify.
        path: DatasetPath,
        /// The [DatasetData] to assign.
        data: DatasetData,
        /// The index into the dataset to assign.
        id: usize
    },

    /// Set the elements after the given id to the given Vec<[DatasetData]>,
    /// in the dataset described by the [DatasetPath]. If the id refers to a
    /// position after the end of the dataset, the dataset will be padded with
    /// [DatasetData::Null].
    SetElements{
        /// The [DatasetPath] to the dataset to modify.
        path: DatasetPath,
        /// A Vec<[DatasetData]> to assign to the dataset
        data: Vec<DatasetData>,
        /// The start index of the assignment.
        id: usize
    },

    /// Remove an element from the dataset described by the [DatasetPath],
    /// shifting all succeding elements back by one.
    DeleteElement{
        /// The [DatasetPath] to the dataset to modify.
        path: DatasetPath,
        /// The index of the element to remove.
        id: usize
    },

    /// Remove all data from the dataset described by the [DatasetPath]
    Empty{
        /// The [DatasetPath] to the dataset to clear.
        path: DatasetPath
    },

    // Dataset Response
    /// The current state of the dataset described by [DatasetPath].
    /// Sent after subscribing to a dataset to synchronize the state on both
    /// ends.
    Dataset{
        /// The [DatasetPath] to the dataset.
        path: DatasetPath,
        /// The [DatasetData] in the dataset.
        data: Vec<DatasetData>
    },
}

impl DatasetData{
    /// Get a nested entry for a [DatasetData::Array] or [DatasetData::Map], if
    /// the DatasetData is an other variant, this will return
    /// [DatasetData::Null]. This function can be called multiple times to
    /// traverse the structure of the data. If it encounters Null, it will
    /// procede without error, only returning Null each time.
    pub fn get_property(&self, property: &String) -> &Self{
        match self{
            DatasetData::Null => &DatasetData::Null,
            DatasetData::Byte(_) => &DatasetData::Null,
            DatasetData::Int(_) => &DatasetData::Null,
            DatasetData::Float(_) => &DatasetData::Null,
            DatasetData::String(_) => &DatasetData::Null,
            DatasetData::Array(arr) => {
                match property.parse::<usize>(){
                    Ok(index) => {
                        match arr.get(index){
                            Some(elem) => elem,
                            None => &DatasetData::Null,
                        }
                    },
                    Err(_) => {
                        &DatasetData::Null
                    },
                }
            },
            DatasetData::Map(map) => {
                match map.get(property) {
                    Some(elem) => elem,
                    None => &DatasetData::Null,
                }
            },
        }
    }

    /// Convert this DatasetData to a string representation.
    pub fn to_string(&self) -> String{
        match self{
            DatasetData::Null => "<null>".to_owned(),
            DatasetData::Byte(b) => b.to_string(),
            DatasetData::Int(i) => i.to_string(),
            DatasetData::Float(f) => f.to_string(),
            DatasetData::String(s) => s.to_string(),
            DatasetData::Array(a) => format!("{:?}", a),
            DatasetData::Map(m) => format!("{:?}", m),
        }
    }
}
use std::{collections::HashMap, fmt::format};

use serde::{Deserialize, Serialize};

use crate::SpiderId2048;



// ========== Absolute Path ==========

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AbsoluteDatasetScope{
	Peripheral(SpiderId2048),
	Public,
}


#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AbsoluteDatasetPath{
	scope: AbsoluteDatasetScope,
    name: Vec<String>,
}

impl AbsoluteDatasetPath{

    pub fn new_public(name: Vec<String>) -> Self{
        Self { 
            scope: AbsoluteDatasetScope::Public,
            name
        }
    }

    pub fn scope(&self) -> &AbsoluteDatasetScope{
        &self.scope
    }
    pub fn parts(&self) -> &Vec<String>{
        &self.name
    }

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DatasetScope{
	Private,
	Public,
}


#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DatasetPath{
	scope: DatasetScope,
    name: Vec<String>,
}

impl DatasetPath{
    pub fn scope(&self) -> &DatasetScope{
        &self.scope
    }
    pub fn parts(&self) -> &Vec<String>{
        &self.name
    }

    pub fn new_private(name: Vec<String>) -> Self {
        Self {
            scope: DatasetScope::Private,
            name,
        }
    }

    pub fn new_public(name: Vec<String>) -> Self {
        Self {
            scope: DatasetScope::Public,
            name,
        }
    }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatasetData{
    Null,
    Byte(u8),
    Int(i32),
    Float(f32),
    String(String),

    Array(Vec<DatasetData>),
    Map(HashMap<String, DatasetData>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatasetMessage{
    // Dataset Requests
	Subscribe{path: DatasetPath},
    
    Append{path: DatasetPath, data: DatasetData},
    Extend{path: DatasetPath, data: Vec<DatasetData>},

    SetElement{path: DatasetPath, data: DatasetData, id: usize},
    SetElements{path: DatasetPath, data: Vec<DatasetData>, id: usize},

    DeleteElement{path: DatasetPath, id: usize},
    // Dataset Response
    Dataset{path: DatasetPath, data: Vec<DatasetData>},
}

impl DatasetData{
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
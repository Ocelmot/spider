use std::{num::NonZeroUsize, path::Path};

use lru::LruCache;
use serde::{ser::SerializeTuple, Deserialize, Deserializer, Serialize, Serializer};
use spider_link::{Relation, Role, SelfRelation};
use tokio::fs;

use crate::error::{ClientResult, ErrorKind, ProblemWrap};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SpiderClientState {
    // Identity
    pub self_relation: SelfRelation,

    #[serde(default)]
    pub host_relation: Option<Relation>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub permission_code: Option<String>,

    // Config
    #[serde(default = "bool_true")]
    pub auto_reconnect: bool,

    // Connection methods
    // Addresses from the Base
    #[serde(default = "bool_true")]
    pub base_addrs_enable: bool,

    #[serde(
        skip_serializing_if = "LruCache::is_empty",
        default = "default_lru",
        serialize_with = "serialize_lru",
        deserialize_with = "deserialize_lru"
    )]
    pub base_addrs: LruCache<String, ()>,

    // Beacon
    #[serde(default = "bool_true")]
    pub beacon_enable: bool,
    #[serde(default = "beacon_default_port")]
    pub beacon_port: u16,

    // Veilid
    #[serde(default)]
    pub veilid_enable: Option<String>,

    #[serde(default)]
    pub veilid_root: Option<String>,

    #[serde(default)]
    pub veilid_own_dht: Option<()>,

    // Fixed Addresses
    #[serde(default)]
    pub fixed_addr_enable: bool,

    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fixed_addrs: Vec<String>,
}

impl SpiderClientState {
    pub fn new(self_rel: SelfRelation) -> Self {
        Self {
            // Identity
            self_relation: self_rel,
            host_relation: None,
            permission_code: None,

            // Config
            auto_reconnect: true,

            // Last Addresses
            base_addrs_enable: true,
            base_addrs: default_lru(),

            // Beacon
            beacon_enable: true,
            beacon_port: beacon_default_port(),

            // Veilid
            veilid_enable: None,
            veilid_own_dht: None,
            veilid_root: None,

            // Fixed Addresses
            fixed_addr_enable: true,
            fixed_addrs: Vec::new(),
        }
    }

    pub fn from_string(s: String) -> ClientResult<Self> {
        serde_json::from_str(&s).wrap_problem(ErrorKind::Deserialize)
    }

    pub async fn from_file<P>(path: P) -> ClientResult<Self>
    where
        P: AsRef<Path> + std::fmt::Debug,
    {
        let data = fs::read_to_string(&path).await.wrap_problem_msg(
            ErrorKind::IO,
            &format!("Failed to read spider config from file: {:?}", path),
        )?;

        serde_json::from_str(&data).wrap_problem(ErrorKind::Deserialize)
    }

    pub async fn to_file(&self, path: &Path) -> ClientResult {
        let data = serde_json::to_string(self).expect("Failed to serialize client state");
        tokio::fs::write(&path, data).await.wrap_problem_msg(
            ErrorKind::IO,
            &format!("Failed to write spider config to file: {:?}", path),
        )
    }
}

impl Default for SpiderClientState {
    fn default() -> Self {
        Self::new(SelfRelation::generate_key(Role::Peripheral))
    }
}

fn default_lru() -> LruCache<String, ()> {
    LruCache::new(NonZeroUsize::new(10).unwrap())
}

fn deserialize_lru<'de, D>(deserializer: D) -> Result<LruCache<String, ()>, D::Error>
where
    D: Deserializer<'de>,
{
    let data: (usize, Vec<String>) = Deserialize::deserialize(deserializer)?;

    let cap = NonZeroUsize::new(data.0).ok_or(serde::de::Error::invalid_value(
        serde::de::Unexpected::Unsigned(data.0 as u64),
        &"lru must have a non-zero length",
    ))?;

    let mut lru = LruCache::new(cap);
    for item in data.1.into_iter().rev() {
        lru.push(item, ());
    }

    Ok(lru)
}

fn serialize_lru<S>(lru: &LruCache<String, ()>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut tup = serializer.serialize_tuple(2)?;
    tup.serialize_element(&lru.len())?;
    let mut vec = Vec::with_capacity(lru.len());
    for (item, _) in lru {
        vec.push(item);
    }
    tup.serialize_element(&vec)?;
    tup.end()
}

fn bool_true() -> bool {
    true
}

pub fn beacon_default_port() -> u16 {
    1930u16
}

pub use spider_link::{
    beacon::{beacon_lookout_many, beacon_lookout_one},
    message, Link, Relation, Role, SelfRelation, SpiderId2048,
};

mod client;
pub use client::{ClientChannel, ClientResponse, SpiderClientBuilder};

mod state;
use state::SpiderClientState;

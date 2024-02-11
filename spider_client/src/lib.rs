#![deny(missing_docs)]

//! The Spider Client library provides functions to manage the connection to the base.
//! 
//! This library also exposes types from the Spider Link create,
//! only one crate should need to be used. 
//! 
//! There are three or four general use patterns for a peripheral client.
//!   - Service - the peripheral process is run as a subprocess of the base.
//!   - Satellite - the peripheral process is run in an embedded environment.
//!   - Standalone - the peripheral process is run as a normal application,
//! but still connects to the base to use its features
//! 
//! A peripheral client may also register as a UI client to render the UI pages
//! for the base. This is similar to the standalone use pattern.
//! 
//! # Usage
//! The client must first be configured and connected to the base in order to be able to processes messages.
//! Once the peripheral is connected, messages can then be sent and recieved.
//! The following two sections show various examples to configure and connect
//! to the base, and send and recieve messages respectively.
//! 
//! ## Configuration and connection examples
//! This example shows how a service peripheral connects to the base.
//! The service peripheral should save its state in a file in the current directory.
//! The service peripheral should always use a fixed address list with the localhost
//! address to connect to the base.
//! The service peripheral can get the key to the base by reading a file called
//! 'spider_keyfile.json' in the directory in which it was run.
//! 
//! ```
//! #[tokio::main]
//! async fn main() {
//!     // Path to saved client state
//!     let client_path = PathBuf::from("client_state.dat");
//!
//!     // Attempt to read the state from the file and apply it to the builder,
//!     // or if it does not exist, create a default builder and pass it to the
//!     // callback to set initial state.
//!     // The callback is only called if the file could not be found.
//!     let mut builder = SpiderClientBuilder::load_or_set(&client_path, |builder| {
//!         // Enable the client to search for the base using addresses from a set list.
//!         builder.enable_fixed_addrs(true);
//!         // Define the list of addrs to search for the base.
//!         builder.set_fixed_addrs(vec!["localhost:1930".into()]);
//!     });
//!
//!     // Load the base's key from a keyfile if it exists.
//!     builder.try_use_keyfile("spider_keyfile.json").await;
//! 
//!     // The channel is then started, and can be used to send and recv messages with the base.
//!     let client_channel = builder.start(true);
//!
//!     println!("Connected");
//! }
//! ```
//! 
//! This example shows how a satellite connects to the base.
//! ```
//! // TODO
//! ```
//! 
//! //! This example shows how a standalone connects to the base.
//! ```
//! // TODO
//! ```
//! 


pub use spider_link::{
    beacon::{beacon_lookout_many, beacon_lookout_one},
    message, Link, Relation, Role, SelfRelation, SpiderId2048,
};

mod client;
pub use client::{ClientChannel, ClientResponse, SpiderClientBuilder};

mod state;
use state::SpiderClientState;

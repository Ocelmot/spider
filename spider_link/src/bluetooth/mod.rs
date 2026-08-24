//! Contains elements of bluetooth connectivity shared by both the base and the
//! clients

use uuid::{uuid, Uuid};

pub mod attach;

/// Identifier for the service in general. Filter by this to find spider bases
pub const BASE: Uuid = uuid!("225c0000-ae12-4fb6-a467-831795f33e94");

/// Read/Notify Characteristic, get the current status from the device
pub const STATUS: Uuid = uuid!("225c0001-ae12-4fb6-a467-831795f33e94");

/// Notify Characteristic, get networks the base can see
pub const NETWORKS: Uuid = uuid!("225c0002-ae12-4fb6-a467-831795f33e94");

/// Read Characteristic, get the current nonce the phone is accepting
pub const NONCE: Uuid = uuid!("225c0003-ae12-4fb6-a467-831795f33e94");

/// Write Characteristic, set the network to which the base should attach
pub const ATTACH: Uuid = uuid!("225c0004-ae12-4fb6-a467-831795f33e94");

//! Implementations of discovery for mDNS protocol
//! 

pub mod advertise;

#[cfg(target_os="ios")]
pub mod discover;
//! The discovery module contains methods for a client to discover potential
//! bases to pair to.
//!
//! It also contains the counterpart code to respond to those queries.

#[cfg(all(feature = "discovery", not(feature = "transport_tcp")))]
compile_error!("feature \"discovery\" requires \"transport_tcp\"");
#[cfg(feature = "discovery")]
pub mod beacon;
#[cfg(feature = "discovery")]
pub mod mdns;

use std::{future::{Future, pending}, pin::Pin};

use link_set::links::Address;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{error::ErrorKind, LinkResult, SpiderId2048};

/// Encodes the discovery or loss of contact of BaseAdverts
#[derive(Debug, Clone)]
pub enum AdvertEvent {
    /// This Advert was newly discovered
    Found(BaseAdvert),
    /// This Advert has not been seen in too long
    Lost(BaseAdvert),
}

/// Contains the result of a received base advertisement.
///
/// Contains a list of addrs, and optionally a name and id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BaseAdvert {
    /// A list of addrs available to connect to the base
    pub addrs: Vec<Address>,
    /// The human readable name for the base,
    pub name: Option<String>,
    /// The id of the base
    pub id: Option<SpiderId2048>,
}

impl BaseAdvert {
    /// Does this BaseAdvert carry any info? or not?
    pub fn is_empty(&self) -> bool {
        self.addrs.is_empty()
            && self.name.as_ref().is_none_or(|n| n.is_empty())
            && self.id.is_none()
    }

    /// Converts the BaseAdvert into a sequence of bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret = Vec::new();

        // Serialize addr list
        let addr_count = self.addrs.len().min(u8::MAX as usize) as u8;

        ret.push(0); // placeholder until actual count is known

        let mut actual_count = 0;
        for addr in &self.addrs[..addr_count as usize] {
            let addr_bytes = addr.to_bytes();
            if addr_bytes.len() > u16::MAX as usize {
                continue;
            }
            let addr_len = (addr_bytes.len() as u16).to_be_bytes();
            ret.extend_from_slice(&addr_len);
            ret.extend_from_slice(&addr_bytes);
            actual_count += 1;
        }
        ret[0] = actual_count;

        // Serialize name
        let name_str = self.name.as_ref().map_or("", |s| s.as_ref());
        let name_len = name_str.floor_char_boundary(u8::MAX as usize);
        let name_bytes = name_str[..name_len].as_bytes();
        ret.push(name_len as u8);
        ret.extend_from_slice(name_bytes);

        // Serialize id
        let id_bytes = self
            .id
            .as_ref()
            .map_or([].as_slice(), |id| id.to_minimal_bytes());
        if id_bytes.len() <= u16::MAX as usize {
            let id_len = (id_bytes.len() as u16).to_be_bytes();
            ret.extend_from_slice(&id_len);
            ret.extend_from_slice(id_bytes);
        } else {
            warn!("Id too long to fit in base advert!");
            ret.extend_from_slice(&[0u8; 2]);
        }

        ret
    }

    /// Restores the BaseAdvert from its sequence of bytes
    pub fn from_bytes(mut bytes: &[u8]) -> LinkResult<Self> {
        let addr_count = bytes.split_off(..1).ok_or(ErrorKind::Deserialization)?[0];
        let mut advert = Self {
            addrs: Vec::new(),
            name: None,
            id: None,
        };

        let _ = (|| {
            // Extract as many addrs as possible
            for _ in 0..addr_count {
                let addr_len = bytes.split_off(..2)?;
                let addr_len = u16::from_be_bytes(addr_len.try_into().unwrap());
                let addr_bytes = bytes.split_off(..addr_len as usize)?;
                match Address::from_bytes(addr_bytes) {
                    Ok(addr) => advert.addrs.push(addr),
                    Err(e) => debug!("Skipping unparsable Address: {}", e),
                }
            }

            // Try to extract name
            let name_len = bytes.split_off(..1)?[0];
            if name_len != 0 {
                let name_bytes = bytes.split_off(..name_len as usize)?;
                advert.name = Some(String::from_utf8_lossy(name_bytes).into_owned());
            }

            // Try to extract id
            let id_len = bytes.split_off(..2)?;
            let id_len = u16::from_be_bytes(id_len.try_into().unwrap());
            if id_len != 0 {
                let id_bytes = bytes.split_off(..id_len as usize)?;
                if let Ok(id_bytes) = id_bytes.try_into() {
                    advert.id = Some(SpiderId2048::from_minimal_bytes(id_bytes));
                } else {
                    debug!("Deserialized BaseAdvert has id with unexpected length");
                }

                if bytes.len() != 0 {
                    debug!(
                        "Deserialized BaseAdvert, but there were extra bytes: {:?}",
                        bytes
                    );
                }
            }

            Some(())
        })();

        Ok(advert)
    }
}

impl Default for BaseAdvert {
    fn default() -> Self {
        Self {
            addrs: Vec::new(),
            name: None,
            id: None,
        }
    }
}

/// This struct is able to return data about nearby bases
pub trait Discoverer: Send {
    /// Gets data about the next available nearby base
    fn next_addr(&mut self) -> Pin<Box<dyn Future<Output = AdvertEvent> + Send + '_>>;
}

impl Discoverer for () {
    fn next_addr(&mut self) -> Pin<Box<dyn Future<Output = AdvertEvent> + Send + '_>> {
        Box::pin(pending())
    }
}

/// Returns an implementation of Discovery appropriate for the current os
pub fn get_discovery(_beacon_port: u16) -> Box<dyn Discoverer> {
    #[cfg(not(feature = "discovery"))]
    return Box::new(());

    #[cfg(all(feature = "discovery", target_os = "ios"))]
    return Box::new(mdns::discover::MdnsDiscoverer::new());

    #[cfg(all(feature = "discovery", not(target_os = "ios")))]
    return {
        let mut beacon = Box::new(beacon::Beacon::new(std::time::Duration::from_secs(5)));
        beacon.set_port(_beacon_port);
        beacon 
    };
}

//! The system module houses utilities with which to interact with the system
//! the base runs on. This is where per-platform code will reside.

use std::{future::Future, pin::Pin};

use spider_link::bluetooth::attach::{AttachDetails, NetworkDetails, SSID, Status};

use crate::error::{ErrorKind, SpiderError, SpiderResult};


#[cfg(target_os="linux")]
mod linux;

#[cfg(target_os="windows")]
mod windows;

/// Result from the attacher returning the new configuration
pub enum AttachEvent{
    Success(AttachDetails),
    Retry,
    Error(SpiderError),
}



pub trait BluetoothController: Send + Sync {
    fn start_attacher(&self, hardware_code: &str) -> Pin<Box<dyn Future<Output = SpiderResult<Box<dyn AttacherHandle>>> + Send + '_>>;
}

pub trait AttacherHandle: Send + Sync {
    fn set_status(&mut self, status: Status);

    fn add_network(&mut self, network: NetworkDetails);
    fn remove_network(&mut self, network: &SSID);

    fn get_net_config(&mut self) -> Pin<Box<dyn Future<Output = AttachEvent> + Send + '_ >>;
    fn stop(self: Box<Self>) -> Pin<Box<dyn Future<Output = SpiderResult> + Send>>;
}

pub async fn get_bluetooth() -> SpiderResult<Box<dyn BluetoothController>> {
    #[cfg(all(feature = "bluetooth_bluer", target_os="linux"))]
    {
        return Ok(Box::new(linux::bluetooth::LinuxBT::create().await?));
    }

    #[allow(unreachable_code)]
    Err(SpiderError::new().problem(ErrorKind::SystemError).msg("No bluetooth implementation for the current platform"))
}

pub enum NetworkEvent{
    Attached,
    Unattached,
}

/// Result from the network manager after attempting to connect to wifi
pub enum ConnectError{
    /// Incorrect password/ sec type
    Credentials,
    /// Could not find the network
    NotFound,
    /// Did not get ip address
    NoAddress,
    /// Transient error
    Retry,
    /// Fatal system error
    System
}

pub trait NetworkController: Send + Sync {
    fn get_network_change(&mut self) -> Pin<Box<dyn Future<Output = SpiderResult<NetworkEvent>> + Send >>;
    fn get_visible_networks(&mut self) -> Pin<Box<dyn Future<Output = SpiderResult<NetworkDetails>> + Send + '_>>;
    fn connect_wifi(&mut self, details: AttachDetails) -> Pin<Box<dyn Future<Output = Result<(), ConnectError>>+Send>>;
}


pub async fn get_network() -> SpiderResult<Box<dyn NetworkController>> {
    #[cfg(all(feature = "network_nmrs", target_os="linux"))]
    {
        return Ok(Box::new(linux::network::LinuxNetwork::create().await?));
    }

    #[allow(unreachable_code)]
    Err(SpiderError::new().problem(ErrorKind::SystemError).msg("No network implementation for the current platform"))
}


use std::{collections::{HashMap, hash_map}, future::Future, pin::Pin, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};

use nmrs::{DeviceState, MonitorHandle, NetworkManager, SettingsPatch, builders::WifiConnectionBuilder, raw::zvariant::Value};
use spider_link::bluetooth::attach::{AttachDetails, AttachSecurityType, NetworkBand, NetworkDetails, NetworkSecurityType};
use tokio::time::{Instant, Sleep, sleep_until};
use tracing::error;

use crate::{
    error::{ErrorKind, ProblemWrap, SpiderResult}, misc::ready_signal::ReadySignal, system::{ConnectError, NetworkController, NetworkEvent},
};

const SPIDER_PROFILE: &'static str = "spider_wifi";
const SPIDER_PROFILE_TEMP: &'static str = "spider_wifi_temp";

pub struct LinuxNetwork {
    nm: NetworkManager,
    // visible networks
    seen_networks: hash_map::IntoIter<Vec<u8>, NetworkDetails>,
    last_seen: Option<Pin<Box<Sleep>>>,
    // network changes
    _monitor_handle: MonitorHandle,
    device_changed: Arc<ReadySignal>,
    attached: Arc<AtomicBool>,
}

impl LinuxNetwork {
    pub async fn create() -> SpiderResult<Self> {
        let nm = NetworkManager::new().await.wrap_problem(ErrorKind::SystemError)?;
        
        // clear old network profiles
        let mut old_timestamp: Option<(u64, String)> = None;
        for connection in nm.list_saved_connections().await.wrap()? {
            if connection.id == SPIDER_PROFILE_TEMP {
                nm.delete_saved_connection(&connection.uuid).await.wrap()?;
            }
            if connection.id == SPIDER_PROFILE {
                match &mut old_timestamp {
                    Some((old_ts, old_uuid)) => {
                        if connection.timestamp_unix > *old_ts {
                            nm.delete_saved_connection(&old_uuid).await.wrap()?;
                            old_timestamp = Some((connection.timestamp_unix, connection.uuid));
                        }else{
                            nm.delete_saved_connection(&connection.uuid).await.wrap()?;
                        }
                    },
                    None => old_timestamp = Some((connection.timestamp_unix, connection.uuid)),
                }
            }
        }


        let device_changed = Arc::new(ReadySignal::new());
        let monitor_device_changed = device_changed.clone();
        let monitor_handle = nm.monitor_device_changes(move || {
            monitor_device_changed.signal();
        }).await.wrap_problem(ErrorKind::SystemError)?;
        device_changed.signal(); // check at least once at startup
        Ok(Self {
            nm,
            seen_networks: HashMap::new().into_iter(),
            last_seen: None,
            _monitor_handle: monitor_handle,
            device_changed,
            attached: Arc::new(AtomicBool::new(true)),
        })
    }
}

impl NetworkController for LinuxNetwork {
    fn get_network_change(&mut self) -> Pin<Box<dyn Future<Output = SpiderResult<NetworkEvent>> + Send>> {
        let device_changed = self.device_changed.clone();
        let nm = self.nm.clone();
        let self_attached = self.attached.clone();
        
        Box::pin(async move {
            loop{
                let guard = device_changed.wait().await;

                // test devices
                let mut has_activated = false;
                for device in nm.list_devices().await.wrap()? {
                    if device.is_wired() || device.is_wireless() {
                        if DeviceState::Activated == device.state {
                            has_activated = true;
                            break;
                        }
                    }
                }

                guard.consume();
                if self_attached.compare_exchange(!has_activated, has_activated, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                    return Ok(if has_activated {
                        NetworkEvent::Attached
                    }else{
                        NetworkEvent::Unattached
                    });
                }
            }
        })
    }

    fn get_visible_networks(&mut self) -> Pin<Box<dyn Future<Output = SpiderResult<NetworkDetails>> + Send + '_>> {
        Box::pin(async{
            loop{
                if let Some((_ssid, network_details)) = self.seen_networks.next() {
                    return Ok(network_details);
                }else{
                    if let Some(s) = &mut self.last_seen {
                        s.await;
                    }
                    let networks = self.nm.list_access_points(None).await.wrap_problem(ErrorKind::SystemError)?;
                    let networks = networks.into_iter().map(|e|{
                        let ssid = e.ssid_bytes;
                        let strength = (e.strength.cast_signed() / 20) * 20;

                        let mut sec_type = NetworkSecurityType::Unknown;
                        if e.security.is_open() {
                            sec_type = NetworkSecurityType::Open
                        }
                        if e.security.wep40 || e.security.wep104 {
                            sec_type = NetworkSecurityType::Wep;
                        }
                        if e.security.psk {
                            sec_type = NetworkSecurityType::WpaPsk;
                        }
                        if e.security.sae {
                            sec_type = NetworkSecurityType::Wpa3Sae;
                        }
                        if e.security.is_enterprise(){
                            sec_type = NetworkSecurityType::Enterprise;
                        }
                        
                        let band = {
                            if e.frequency_mhz <= 2500 {
                                NetworkBand::Band2_4
                            }else if e.frequency_mhz <= 5100{
                                NetworkBand::Band5
                            }else{
                                NetworkBand::Band6
                            }
                        };
                        NetworkDetails::new(ssid, strength, sec_type, band).expect("SSID is not greater than 32 bytes")
                    }).fold(HashMap::<Vec<u8>, NetworkDetails>::new(), |mut map, element|{
                        match map.get_mut(&element.ssid) {
                            Some(existing) => {
                                if element.strength > existing.strength {
                                    *existing = element;
                                }
                            }
                            None => {
                                map.insert(element.ssid.clone(), element);
                            }
                        }
                        map
                    });
                    
                    self.seen_networks = networks.into_iter();
                    self.last_seen = Some(Box::pin(sleep_until(Instant::now() + Duration::from_secs(2))));
                    let _ = self.nm.scan_networks(None).await;
                }
            }
        })
    }

    fn connect_wifi(
        &mut self,
        details: AttachDetails,
    ) -> Pin<Box<dyn Future<Output = Result<(), ConnectError>> + Send>> {
        let nm = self.nm.clone();
        Box::pin(async move{
            // Set invariants, get initial state
            if let Some(uuid) = nm.get_saved_connection_uuid(SPIDER_PROFILE_TEMP).await.map_err(|_|ConnectError::Retry)? {
                let _ = nm.delete_saved_connection(&uuid).await;
            }
            let interface = nm.list_wifi_devices().await.map_err(|_|ConnectError::System)?.into_iter().next().ok_or_else(||ConnectError::System)?.interface;
            let old_connection = nm.list_saved_connections().await.map_err(|_|ConnectError::Retry)?.into_iter().find(|c| c.id == SPIDER_PROFILE);

            let sec_type = details.sec_type();
            // If this is an auto and not hidden network, find its encryption type
            let lossy_ssid = String::from_utf8_lossy(details.ssid());

            // attempt connection
            let mut settings = WifiConnectionBuilder::new(lossy_ssid)
                .autoconnect(false)
                .ipv4_auto()
                .ipv6_auto()
                .hidden(details.hidden());

            settings = match sec_type.clone() {
                AttachSecurityType::Open => settings.open(),
                | AttachSecurityType::Wpa2Personal(psk)
                | AttachSecurityType::Wpa3Personal(psk) => settings.wpa_psk(psk),
            };
            

            let mut settings = settings.build();
            settings.get_mut("connection").expect("key is set by connection builder").insert("id", Value::from(SPIDER_PROFILE_TEMP));
            settings.get_mut("802-11-wireless").expect("key is set by connection builder").insert("ssid", Value::from(details.ssid().to_owned()));
            if let AttachSecurityType::Wpa3Personal(_) = sec_type {
                let sec = settings.get_mut("802-11-wireless-security").expect("key is set by connection builder");
                sec.insert("key-mgmt", Value::from("sae"));
                sec.remove("auth-alg");
            }

            match nm.add_and_activate_connection(settings, Some(&interface), None).await{
                Ok((_profile, _active)) => {
                    if let Some(new_uuid) = nm.get_saved_connection_uuid(SPIDER_PROFILE_TEMP).await.map_err(|_|ConnectError::System)? {
                        let mut patch = SettingsPatch::default();
                        patch.autoconnect = Some(true);
                        patch.id = Some(String::from(SPIDER_PROFILE));

                        nm.update_saved_connection(&new_uuid, patch).await.map_err(|_|ConnectError::Retry)?;
                    }

                    if let Some(old_connection) = old_connection {
                        let _ = nm.delete_saved_connection(&old_connection.uuid).await;
                    }

                    Ok(())
                },
                Err(e) => {
                    if let Ok(Some(new_uuid)) = nm.get_saved_connection_uuid(SPIDER_PROFILE_TEMP).await {
                        let _ = nm.delete_saved_connection(&new_uuid).await;
                    }

                    let _ = nm.set_wifi_enabled(&interface, true).await;

                    match e {
                        nmrs::ConnectionError::NotFound => Err(ConnectError::NotFound),
                        nmrs::ConnectionError::AuthFailed => Err(ConnectError::Credentials),
                        nmrs::ConnectionError::SupplicantTimeout => Err(ConnectError::Credentials),
                        nmrs::ConnectionError::DhcpFailed => Err(ConnectError::NoAddress),
                        nmrs::ConnectionError::Timeout => Err(ConnectError::Retry),
                        nmrs::ConnectionError::NoWifiDevice => Err(ConnectError::System),
                        nmrs::ConnectionError::WifiNotReady => Err(ConnectError::System),
                        nmrs::ConnectionError::IncompleteBuilder(_) => Err(ConnectError::System),
                        nmrs::ConnectionError::MissingPassword => Err(ConnectError::Credentials),
                        nmrs::ConnectionError::InvalidUtf8(_utf8_error) => Err(ConnectError::System),
                        nmrs::ConnectionError::ApBssidNotFound { ssid: _, bssid: _ } => Err(ConnectError::NotFound),
                        nmrs::ConnectionError::NotAWifiDevice { interface: _ } => Err(ConnectError::System),
                        nmrs::ConnectionError::WifiInterfaceNotFound { interface: _ } => Err(ConnectError::System),
                        nmrs::ConnectionError::HardwareRadioKilled => Err(ConnectError::System),
                        nmrs::ConnectionError::InvalidInput { field: _, reason: _ } => Err(ConnectError::System),
                        nmrs::ConnectionError::SupplicantConfigFailed => Err(ConnectError::System),
                        _ => {
                            error!("connect_wifi encountered unknown error: {}", e);
                            Err(ConnectError::Retry)
                        },
                    }
                },
            }
        })
    }
}

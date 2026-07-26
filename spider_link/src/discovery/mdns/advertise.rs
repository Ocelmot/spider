//! Implementations of discovery for mDNS protocol
//!
//!

use std::{format, time::Duration};

use crate::discovery::BaseAdvert;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio::{sync::watch, task::JoinHandle, time::sleep};
use tracing::{debug, info, warn};

/// Implementation of advertiser for mdns
pub fn start_mdns_advertiser(
    mut template: watch::Receiver<BaseAdvert>,
    advert_port: u16,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Ok(mdns) = ServiceDaemon::new() else {
            warn!("mDNS ServiceDaemon failed to start");
            return;
        };
        template.mark_changed();
        let mut service_name: Option<String> = None;
        loop {
            sleep(Duration::from_millis(150)).await;
            let res = template.changed().await;
            if res.is_err() {
                break;
            }
            let advert_template = template.borrow_and_update().clone();

            let name = advert_template.name.as_ref().map_or("NoName", |s| {
                if s.is_empty() {
                    "NoName"
                } else {
                    s.as_str()
                }
            });
            let name = &name[..name.floor_char_boundary(45)];

            let sig = sha256::digest(advert_template.to_bytes());
            let sig = &sig[0..6];

            let instance_name = format!("spider-{}-{}", name, sig);
            let hostname = format!("spider-{}.local.", sig);

            // build properties
            let mut properties = Vec::new();

            // Include addrs
            let mut i = 0;
            for addr in &advert_template.addrs {
                let value = format!("{}", addr);
                if value.as_bytes().len() < 250 {
                    properties.push((format!("addr{i}"), value));
                    i += 1;
                } else {
                    info!("Address skipped, too long: {}", value);
                }
            }

            // Include name
            if let Some(name) = advert_template.name {
                properties.push((
                    "name".to_owned(),
                    name[..name.floor_char_boundary(250)].to_owned(),
                ));
            }

            // Include id
            let encoded_id = advert_template.id.map(|id| id.to_base64());
            if let Some(id) = &encoded_id {
                if id.len() <= 400 && id.len() > 200 {
                    let (id0, id1) = id.split_at(200);
                    properties.push(("id0".to_owned(), id0.to_owned()));
                    properties.push(("id1".to_owned(), id1.to_owned()));
                } else {
                    warn!("Id base64 longer than 400 chars or shorter than 200");
                }
            };

            let Ok(service) = ServiceInfo::new(
                "_spider._tcp.local.",
                &instance_name,
                &hostname,
                "", // Ip
                advert_port,
                properties.as_slice(),
            ) else {
                info!("mDNS failed to set/update service, skipping");
                continue;
            };
            let service = service.enable_addr_auto();

            let new_service_name = service.get_fullname().to_owned();
            if let Err(e) = mdns.register(service) {
                info!("mDNS failed to register new service: {}", e);
                continue;
            };
            if let Some(service_name) = &service_name {
                if *service_name != new_service_name {
                    let _ = mdns.unregister(service_name);
                }
            }
            service_name = Some(new_service_name);
        }
        loop {
            match mdns.shutdown() {
                Ok(_) => break,
                Err(mdns_sd::Error::Again) => sleep(Duration::from_millis(50)).await,
                Err(e) => {
                    warn!("mDNS shutdown: {e}");
                    break;
                }
            }
        }
        debug!("mDNS shut down");
    })
}

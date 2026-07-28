//! mDNS implmention for Discover

use std::{
    collections::HashMap,
    format,
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    time::Duration,
};

use link_set::links::Address;
use tokio::time::{sleep_until, Instant, Sleep};
use tracing::{debug, info, warn};
use zeroconf_tokio::{
    prelude::TMdnsBrowser, txt_record::TTxtRecord, BrowserEvent, MdnsBrowser, MdnsBrowserAsync,
    ServiceType,
};

use crate::{
    discovery::{AdvertEvent, BaseAdvert, Discoverer},
    transports::tcp::TCP_SCHEME,
    SpiderId2048,
};

static ERROR_COUNT_MAX: u8 = 5;

/// MdnsDiscoverer is for the implementation of Discoverer for the mDNS protocol
pub struct MdnsDiscoverer {
    browser: Option<MdnsBrowserAsync>,
    seen_adverts: HashMap<String, BaseAdvert>,
    lost_adverts: HashMap<String, BaseAdvert>,
    error_count: u8,
    sleep: Option<Pin<Box<Sleep>>>,
}

impl MdnsDiscoverer {
    pub(crate) fn new() -> Self {
        Self {
            browser: None,
            seen_adverts: HashMap::new(),
            lost_adverts: HashMap::new(),
            error_count: 0,
            sleep: None,
        }
    }

    fn record_error(&mut self) {
        self.error_count = u8::min(self.error_count + 1, ERROR_COUNT_MAX);
        let deadline = Instant::now() + Duration::from_secs(1 << self.error_count);
        self.sleep = Some(Box::pin(sleep_until(deadline)));
    }
}

impl Discoverer for MdnsDiscoverer {
    fn next_addr(&mut self) -> Pin<Box<dyn Future<Output = AdvertEvent> + Send + '_>> {
        let fut = async {
            loop {
                // First return lost addrs if they exist
                let lost = self.lost_adverts.extract_if(|_, _| true).next();
                if let Some((_name, lost)) = lost {
                    return AdvertEvent::Lost(lost);
                }
                // If there was an error, sleep for retry backoff
                if let Some(sleep) = &mut self.sleep {
                    sleep.await;
                    self.sleep = None;
                }
                // Get new events from the browser
                if let Some(browser) = &mut self.browser {
                    while let Some(event) = browser.next().await {
                        if event.is_ok() {
                            self.error_count = 0;
                        }
                        match event {
                            Ok(BrowserEvent::Add(discovery)) => {
                                // Build Addrs
                                let mut addrs = Vec::new();
                                if let Ok(addr) = discovery.address().parse::<IpAddr>() {
                                    if !addr.is_multicast() && !addr.is_unspecified() {
                                        addrs.push(Address::new(
                                            TCP_SCHEME,
                                            SocketAddr::from((addr, *discovery.port())),
                                        ));
                                    }
                                }

                                let mut name = None;
                                let mut id = None;

                                if let Some(txt) = discovery.txt() {
                                    let mut i = 0u32;
                                    loop {
                                        let key = format!("addr{i}");
                                        if let Some(val) = txt.get(&key) {
                                            i += 1;
                                            if let Ok(address) = val.parse() {
                                                addrs.push(address);
                                            } else {
                                                info!("Address failed to parse");
                                            }
                                        } else {
                                            break;
                                        }
                                    }

                                    name = txt.get("name");
                                    if let Some(id0) = txt.get("id0") {
                                        if let Some(id1) = txt.get("id1") {
                                            let str_id = format!("{}{}", id0, id1);
                                            id = SpiderId2048::from_base64(str_id);
                                        }
                                    }
                                }

                                let advert = BaseAdvert { addrs, name, id };
                                let old = self.seen_adverts
                                    .insert(discovery.name().clone(), advert.clone());
                                match old {
                                    Some(old) => {
                                        if old != advert {
                                            // this will not overwrite since the lost addrs is
                                            // fully drained by the time this executes
                                            self.lost_adverts.insert(discovery.name().clone(), old);
                                            return AdvertEvent::Found(advert);
                                        }
                                    }
                                    None => {
                                        return AdvertEvent::Found(advert);
                                    }
                                }
                            }
                            Ok(BrowserEvent::Remove(removal)) => {
                                if let Some(advert) = self.seen_adverts.remove(removal.name()) {
                                    return AdvertEvent::Lost(advert);
                                } else {
                                    debug!("Errant removal from {}", removal.name());
                                }
                            }
                            Err(e) => {
                                debug!("mDNS browser encountered error: {}", e);
                            }
                        }
                    }
                    warn!("mDNS browser shut down");
                    self.lost_adverts.extend(self.seen_adverts.drain());
                    let _ = browser.shutdown().await;
                    self.browser = None;
                    self.record_error();
                } else {
                    let service_type = ServiceType::new("spider", "tcp").unwrap();
                    let mut browser = MdnsBrowserAsync::new(MdnsBrowser::new(service_type))
                        .expect("Operation cannot fail");
                    match browser.start().await {
                        Ok(_) => {
                            self.browser = Some(browser);
                        }
                        Err(e) => {
                            warn!("Encountered error starting mDNS: {}", e);
                            self.record_error();
                        }
                    }
                }
            }
        };
        Box::pin(fut)
    }
}

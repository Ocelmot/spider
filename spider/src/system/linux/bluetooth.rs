//! The linux implementations of system controller traits
//!

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use bluer::{
    adv::{Advertisement, AdvertisementHandle},
    gatt::local::{
        characteristic_control, Application, ApplicationHandle, Characteristic,
        CharacteristicControlEvent, CharacteristicNotify, CharacteristicNotifyMethod,
        CharacteristicRead, CharacteristicWrite, Service,
    },
    Adapter, Session,
};
use futures::StreamExt;
use spider_link::bluetooth::{
    ATTACH, BASE, NETWORKS, NONCE, STATUS, attach::{
        AttachDetails, AttachKey, AttachSalt, NetworkDetails, SSID, Status, generate_key, generate_nonce,
    },
};
use tokio::{
    select,
    sync::{
        mpsc::{channel, Receiver},
        watch, Mutex,
    },
    time::sleep,
};
use tracing::{debug, warn};

use crate::{
    error::{ErrorKind, ProblemWrap, SpiderError, SpiderResult},
    system::{AttachEvent, AttacherHandle, BluetoothController},
};

pub struct LinuxBT {
    _session: Session,
    adapter: Adapter,
    salt: AttachSalt,
}

impl LinuxBT {
    pub async fn create() -> SpiderResult<LinuxBT> {
        let session = Session::new().await.wrap_problem(ErrorKind::SystemError)?;
        let adapter = session
            .default_adapter()
            .await
            .wrap_problem(ErrorKind::SystemError)?;
        adapter
            .set_powered(true)
            .await
            .wrap_msg("Failed to set power on")?;
        let salt = rand::random();
        Ok(Self {
            _session: session,
            adapter,
            salt,
        })
    }
}

impl BluetoothController for LinuxBT {
    fn start_attacher(
        &self,
        hardware_code: &str,
    ) -> Pin<Box<dyn Future<Output = SpiderResult<Box<dyn AttacherHandle>>> + Send + '_>> {
        let key = generate_key(hardware_code, &self.salt).wrap();
        Box::pin(async move {
            let key = key?;
            LinuxAH::start(&self.adapter, key, self.salt)
                .await
                .map(|handle| Box::new(handle) as Box<dyn AttacherHandle>)
        })
    }
}

struct LinuxAH {
    _adv_handle: AdvertisementHandle,
    _app_handle: ApplicationHandle,

    status_tx: watch::Sender<Status>,
    networks_tx: watch::Sender<HashMap<SSID, NetworkDetails>>,
    details_rx: Receiver<AttachEvent>,
}

impl LinuxAH {
    async fn start(adapter: &Adapter, key: AttachKey, salt: AttachSalt) -> SpiderResult<Self> {
        let adv_handle = adapter
            .advertise(Advertisement {
                service_uuids: vec![BASE].into_iter().collect(),
                local_name: Some("spider".into()),
                discoverable: Some(true),
                ..Default::default()
            })
            .await
            .wrap_msg("failed to advertise on adapter")?;

        let (status_tx, status_rx) = watch::channel(Status::Ready);
        let status_rx_read = status_rx.clone();
        let status_rx_notify = status_rx;

        let (networks_tx, networks_rx) = watch::channel(HashMap::<SSID, NetworkDetails>::new());
        let (mut networks_control, networks_control_handle) = characteristic_control();

        let nonce = Arc::new(Mutex::new(None));
        let nonce_status_tx = status_tx.clone();

        let (details_tx, details_rx) = channel::<AttachEvent>(50);
        let details_nonce = nonce.clone();

        let app_handle = adapter
            .serve_gatt_application(Application {
                services: vec![Service {
                    uuid: BASE,
                    primary: true,
                    characteristics: vec![
                        // STATUS
                        Characteristic {
                            uuid: STATUS,
                            read: Some(CharacteristicRead {
                                read: true,

                                fun: Box::new(move |_req| {
                                    let rx = status_rx_read.clone();
                                    Box::pin(async move {
                                        let val = rx.borrow().to_bytes();
                                        Ok(val)
                                    })
                                }),
                                ..Default::default()
                            }),
                            notify: Some(CharacteristicNotify {
                                notify: true,

                                method: CharacteristicNotifyMethod::Fun(Box::new(
                                    move |mut notifier| {
                                        let mut rx = status_rx_notify.clone();
                                        Box::pin(async move {
                                            tokio::spawn(async move {
                                                loop {
                                                    select! {
                                                        _ = notifier.stopped() => {break;} // Connection closed
                                                        res = rx.changed() => {
                                                            if res.is_err(){
                                                                break;
                                                            }

                                                            let status = {
                                                                rx.borrow_and_update().to_bytes()
                                                            };

                                                            let res = notifier.notify(status).await;
                                                            if res.is_err(){
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                        })
                                    },
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        // NETWORKS
                        Characteristic {
                            uuid: NETWORKS,
                            notify: Some(CharacteristicNotify {
                                notify: true,
                                method: CharacteristicNotifyMethod::Io,
                                ..Default::default()
                            }),
                            control_handle: networks_control_handle,
                            ..Default::default()
                        },
                        Characteristic {
                            uuid: NONCE,
                            read: Some(CharacteristicRead {
                                read: true,

                                fun: Box::new(move |_req| {
                                    let local_nonce = nonce.clone();
                                    let local_status_tx = nonce_status_tx.clone();
                                    Box::pin(async move {
                                        let new_nonce = generate_nonce();
                                        let mut lock = local_nonce.lock().await;
                                        *lock = Some(new_nonce.clone());
                                        local_status_tx.send_replace(Status::Ready);
                                        let nonce_bytes = new_nonce.to_vec();
                                        let bytes = [salt.as_slice(), nonce_bytes.as_slice()].concat();
                                        Ok(bytes)
                                    })
                                }),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        Characteristic {
                            uuid: ATTACH,
                            write: Some(CharacteristicWrite {
                                write: true,
                                method: bluer::gatt::local::CharacteristicWriteMethod::Fun(
                                    Box::new(move |bytes, _req| {
                                        Box::pin({
                                            let inner_tx = details_tx.clone();
                                            let key = key.clone();
                                            let local_nonce = details_nonce.clone();
                                            async move {
                                                let nonce = local_nonce.lock().await.take();
                                                if let Some(nonce) = nonce {
                                                    match AttachDetails::unpack(bytes, &key, &nonce)
                                                    {
                                                        Ok(s) => {
                                                            let _ = inner_tx
                                                                .send(AttachEvent::Success(s))
                                                                .await;
                                                        }
                                                        Err(e) => {
                                                            debug!(
                                                                "Unpack encountered error: {}",
                                                                e
                                                            );
                                                            let _ = inner_tx
                                                                .send(AttachEvent::Retry)
                                                                .await;
                                                        }
                                                    }
                                                } else {
                                                    let _ = inner_tx.send(AttachEvent::Retry).await;
                                                }
                                                Ok(())
                                            }
                                        })
                                    }),
                                ),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .wrap_msg("Failed to start gatt app")?;

        // Setup tasks
        tokio::spawn(async move {
            while let Some(event) = networks_control.next().await{
                let CharacteristicControlEvent::Notify(writer) = event else{continue};
                let mut rx = networks_rx.clone();
                rx.mark_changed();

                tokio::spawn(async move {
                    let mut details: Vec<NetworkDetails> = Vec::new();
                    loop {
                        select! {
                            _ = writer.closed() => {break;} // Connection closed
                            res = rx.changed() => {
                                if res.is_err(){
                                    break;
                                }
                                let x = rx.borrow_and_update();
                                details = x.values().cloned().collect();
                            }
                            stopped = async {
                                for detail in &details {
                                    let msg = detail.to_bytes();
                                    if msg.len() > writer.mtu() {
                                        warn!("{} exceeds mtu {}", String::from_utf8_lossy(&detail.ssid), writer.mtu());
                                        continue;
                                    }
                                    let res = writer.send(&msg).await;
                                    if res.is_err(){
                                        return true;
                                    }
                                }
                                sleep(Duration::from_millis(1500)).await;
                                false
                            } => {
                                if stopped {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });

        // Return Struct
        Ok(Self {
            _adv_handle: adv_handle,
            _app_handle: app_handle,
            status_tx,
            networks_tx,
            details_rx,
        })
    }
}

impl AttacherHandle for LinuxAH {
    fn set_status(&mut self, status: Status) {
        self.status_tx.send_replace(status);
    }

    /// Add this network to the list of advertised networks
    fn add_network(&mut self, network: NetworkDetails) {
        self.networks_tx.send_if_modified(|networks| {
            if let Some(details) = networks.get_mut(&network.ssid) {
                if *details != network {
                    *details = network;
                    true
                } else {
                    false
                }
            } else {
                networks.insert(network.ssid.clone(), network);
                true
            }
        });
    }

    /// Remove this network from the list of advertised networks
    fn remove_network(&mut self, network: &SSID) {
        self.networks_tx
            .send_if_modified(|networks| networks.remove(network).is_some());
    }

    /// Get the set network configuration from the GATT server
    fn get_net_config(&mut self) -> Pin<Box<dyn Future<Output = AttachEvent> + Send + '_>> {
        Box::pin(async {
            loop {
                match self.details_rx.recv().await {
                    Some(val) => return val,
                    None => {
                        return AttachEvent::Error(SpiderError::new().problem(ErrorKind::Stopped))
                    }
                }
            }
        })
    }

    /// Shutdown and cleanup the attacher
    fn stop(self: Box<Self>) -> Pin<Box<dyn Future<Output = SpiderResult> + Send>> {
        Box::pin(async {
            // Give the success message a chance to propagate
            sleep(Duration::from_millis(1000)).await;
            // Dropping the handles is enough to clean up the gatt server
            Ok(())
        })
    }
}

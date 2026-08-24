use std::collections::HashMap;

use spider_link::{
    bluetooth::attach::{NetworkDetails, Status},
    opt_await::OrPend,
};
use tokio::{select, sync::mpsc::Receiver, task::JoinHandle};
use tracing::{error, info};

use crate::{
    error::{ErrorKind, SpiderError, SpiderResult},
    processor::{link::ProcessorLink, system::SystemProcessorMessage},
    system::{
        get_bluetooth, get_network, AttachEvent, AttacherHandle, BluetoothController,
        NetworkController, NetworkEvent,
    },
};

const NETWORK_TTL: u8 = 3;

pub(crate) struct SystemProcessorState {
    pl: ProcessorLink,
    receiver: Receiver<SystemProcessorMessage>,

    bt: Option<Box<dyn BluetoothController>>,
    attacher: Option<Box<dyn AttacherHandle>>,

    network_controller: Option<Box<dyn NetworkController>>,
    /// map from ssid to ttl
    known_networks: HashMap<Vec<u8>, (NetworkDetails, u8)>,
}

impl SystemProcessorState {
    pub(crate) fn new(pl: ProcessorLink, receiver: Receiver<SystemProcessorMessage>) -> Self {
        Self {
            pl,
            receiver,

            bt: None,
            attacher: None,

            network_controller: None,
            known_networks: HashMap::new(),
        }
    }

    pub(crate) fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            match self.init().await {
                Ok(_) => {
                    info!("System processor init succeeded")
                }
                Err(e) => info!("System processor encountered error initializing: {}", e.get_trace()),
            }

            loop {
                select! {
                    msg = self.receiver.recv() => {
                        let Some(msg) = msg else{break};
                        let _ = self.handle_message(msg).await;
                    }
                    event = self.attacher.as_deref_mut().map(|a|a.get_net_config()).or_pend() => {
                        let attacher = self.attacher.as_deref_mut().expect("none would pend");
                        match event{
                            AttachEvent::Success(attach_details) => {
                                if let Some(network_controller) = self.network_controller.as_deref_mut(){
                                    attacher.set_status(Status::Attaching);
                                    match network_controller.connect_wifi(attach_details).await {
                                        Ok(_) => attacher.set_status(Status::Attached),
                                        Err(e) => match e {
                                            crate::system::ConnectError::Credentials => attacher.set_status(Status::InvalidCredentials),
                                            crate::system::ConnectError::NotFound => attacher.set_status(Status::NotFound),
                                            crate::system::ConnectError::NoAddress => attacher.set_status(Status::NoAddress),
                                            crate::system::ConnectError::Retry => attacher.set_status(Status::Retry),
                                            crate::system::ConnectError::System => attacher.set_status(Status::SystemError),
                                        },
                                    }
                                }
                            },
                            AttachEvent::Retry => attacher.set_status(Status::Retry),
                            AttachEvent::Error(e) => {  // Attacher stopped
                                error!("Attacher encountered error: {e}");
                                let _ = self.stop_advert().await;
                                let _ = self.start_advert().await;
                            },
                        }
                    }
                    event = self.network_controller.as_deref_mut().map(|c|c.get_network_change()).or_pend() => {
                        match event {
                            Ok(NetworkEvent::Attached) => {
                                let _ = self.stop_advert().await;
                            }
                            Ok(NetworkEvent::Unattached) => {
                                if let Err(e) = self.start_advert().await {
                                    error!("Attacher encountered error: {e}");
                                }
                            }
                            Err(e) => error!("System encountered error: {}", e),
                        }
                    }
                    event = self.network_controller.as_deref_mut().map(|c|c.get_visible_networks()).or_pend(), if self.attacher.is_some() => {
                        match event {
                            Ok(event) => {
                                self.known_networks.insert(event.ssid.clone(), (event.clone(), NETWORK_TTL));
                                if let Some(attacher) = &mut self.attacher {
                                    attacher.add_network(event);
                                }
                            },
                            Err(e) => error!("System Encountered error: {}", e),
                        }

                    }
                };
            }
            let _ = self.stop_advert().await;
        });
        handle
    }

    async fn init(&mut self) -> SpiderResult {
        self.bt = match get_bluetooth().await{
            Ok(bt) => Some(bt),
            Err(e) => {
                error!("Failed to start bluetooth: {e}");
                None
            },
        };
        self.network_controller = match get_network().await {
            Ok(network_controller) => Some(network_controller),
            Err(e) => {
                error!("Failed to start network: {e}");
                None
            },
        };
        Ok(())
    }

    /// Start the attacher, if the attacher is already running this does
    /// nothing.
    async fn start_advert(&mut self) -> SpiderResult {
        if self.attacher.is_none() {
            let bt = self
                .bt
                .as_ref()
                .ok_or(SpiderError::new().problem(ErrorKind::Uninitialized))?;
            let hardware_code = self.pl.hardware_code();
            let mut attacher = bt.start_attacher(hardware_code).await?;
            for (_ssid, (details, _ttl)) in &mut self.known_networks {
                attacher.add_network(details.to_owned());
            }
            self.attacher = Some(attacher);
        }
        Ok(())
    }

    async fn stop_advert(&mut self) -> SpiderResult {
        if let Some(attacher) = self.attacher.take() {
            attacher.stop().await?;
        }
        Ok(())
    }

    async fn handle_message(&mut self, msg: SystemProcessorMessage) -> SpiderResult<()> {
        if !matches!(msg, SystemProcessorMessage::Upkeep) {
            info!("System message: {:?}", msg);
        }
        match msg {
            SystemProcessorMessage::Upkeep => {
                for (expired, _) in self.known_networks.extract_if(|_key, (_, ttl)| {
                    *ttl = ttl.saturating_sub(1);
                    *ttl == 0
                }) {
                    if let Some(attacher) = &mut self.attacher {
                        attacher.remove_network(&expired);
                    }
                }
                Ok(())
            }
        }
    }
}

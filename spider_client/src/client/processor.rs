use core::panic;
use std::{path::PathBuf, time::Duration};

use dht_chord::{
    adaptor::{AssociateClient, ChordAdaptor},
    TCPAdaptor,
};
use spider_link::{
    beacon::beacon_lookout_one,
    message::{Message, RouterMessage},
    Link, SpiderId2048,
};
use tokio::{
    select, spawn,
    sync::mpsc::{channel, error::SendError, Receiver, UnboundedSender, unbounded_channel},
    task::JoinHandle,
    time::{sleep, timeout},
};

use crate::{state::SpiderClientState, SpiderClientBuilder};

use super::{channel::ClientChannel, ClientControl, ClientResponse};

pub struct SpiderClientProcessor {
    state_path: Option<PathBuf>,
    state: SpiderClientState,
    terminate: bool,
    client_channel: ClientChannel,
    receiver: Receiver<ClientControl>,
    link: Option<Link>,
    on_message: Option<Box<dyn FnMut(&ClientChannel, Message) + Send>>,
    on_connect: Option<Box<dyn FnMut(&ClientChannel) + Send>>,
    on_disconnect: Option<Box<dyn FnMut(&ClientChannel) + Send>>,
    on_terminate: Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>,
    on_deny: Option<Box<dyn FnMut(SpiderClientBuilder) + Send>>,
    channels: Vec<UnboundedSender<ClientResponse>>,
}

impl SpiderClientProcessor {
    pub(crate) fn start(
        state_path: Option<PathBuf>,
        state: SpiderClientState,
        enable_recv: bool
    ) -> (ClientChannel, JoinHandle<()>) {
        if state.host_relation.is_none() {
            panic!("Processor requires a host to be able to connect");
        }
        let id = state.self_relation.relation.id.clone();

        let (sender, receiver) = channel(50);
        let (client_channel, channels) = if enable_recv {
            let (tx, rx) = unbounded_channel();
            let client_channel = ClientChannel::with_receiver(id, sender, rx);
            (client_channel, vec![tx])
        } else {
            let client_channel = ClientChannel::new(id, sender);
            (client_channel, Vec::new())
        };

        let mut processor = Self {
            state_path,
            state,
            terminate: false,
            client_channel: client_channel.clone(),
            receiver,
            link: None,
            on_message: None,
            on_connect: None,
            on_disconnect: None,
            on_terminate: None,
            on_deny: None,
            channels,
        };

        let handle = spawn(async move {
            // if the processor is disconnected, attempt to connect
            // if the processor is connected, process messages
            while !processor.terminate {
                match processor.link {
                    Some(ref mut link) => {
                        select! {
                            msg = processor.receiver.recv() => {
                                // new message from users
                                match msg {
                                    Some(msg) => {
                                        processor.process_client_control(msg).await;
                                    },
                                    // since the processor itself holds a
                                    // sender to the channel, this should not occur
                                    None => break,
                                }
                            },
                            msg = link.recv() => {
                                // new message from base
                                match msg {
                                    Some(msg) => {
                                        if let Message::Router(RouterMessage::Pending) = &msg {
                                            // if we are pending, send saved permission code
                                            if let Some(code) = &processor.state.permission_code {
                                                let msg = RouterMessage::ApprovalCode(code.clone());
                                                let msg = Message::Router(msg);
                                                link.send(msg).await;
                                            }
                                        }
                                        if let Message::Router(RouterMessage::Denied) = &msg {
                                            let mut builder = SpiderClientBuilder {
                                                state_path: processor.state_path.clone(),
                                                state: processor.state.clone(),
                                            };
                                            // Since the connection is denied, remove the host relation
                                            builder.state.host_relation = None;
                                            processor.process_client_response(ClientResponse::Denied(builder)).await;
                                            return;
                                        }
                                        if let Message::Router(RouterMessage::ChordAddrs(addrs)) = &msg {
                                            println!("RECVD CHORD ADDRS: {:?}", addrs);
                                            processor.state.chord_addrs = addrs.clone();
                                            processor.save_state();
                                        }
                                        processor.process_client_response(ClientResponse::Message(msg)).await
                                    },
                                    None => {
                                        // became disconected
                                        processor.link = None;
                                        processor.process_client_response(ClientResponse::Disconnected).await
                                    },
                                }
                            }
                        }
                    }
                    None => {
                        // reconnect
                        if let Some(addr) = processor.connect().await {
                            if processor.state.last_addr_enable {
                                processor.state.set_last_addr(Some(addr));
                                processor.save_state();
                            }
                        }

                        // establish reconnection subscriptions
                        if processor.state.chord_enable {
                            match processor
                                .link_send(Message::Router(RouterMessage::SubscribeChord(50)))
                                .await
                            {
                                Ok(_) => {}
                                Err(_) => continue, // couldnt send message, need to reconnect
                            }
                        }

                        processor.process_client_response(ClientResponse::Connected).await;
                    }
                }
            }

            // On terminate
            let builder = SpiderClientBuilder {
                state_path: processor.state_path.clone(),
                state: processor.state.clone(),
            };
            let msg = ClientResponse::Terminated(builder);
            processor.process_client_response(msg).await;
        });
        (client_channel, handle)
    }

    fn save_state(&self){
        if let Some(path) = &self.state_path {
            self.state.to_file(path)
        }
    }

    async fn process_client_control(&mut self, msg: ClientControl) {
        match msg {
            ClientControl::Message(msg) => {
                self.link_send(msg).await;
            }
            ClientControl::AddChannel(ch) => {
                self.channels.push(ch);
            }
            ClientControl::SetOnMessage(cb) => {
                self.on_message = cb;
            }
            ClientControl::SetOnConnect(cb) => {
                self.on_connect = cb;
            }
            ClientControl::SetOnTerminate(cb) => {
                self.on_terminate = cb;
            }
            ClientControl::SetOnDeny(cb) => {
                self.on_deny = cb;
            }
            ClientControl::Terminate => {
                self.terminate = true;
            }
        };
    }

    async fn link_send(&mut self, msg: Message) -> Result<(), SendError<Message>> {
        // shouldnt be able to be none at this point, but lets be safe.
        if let Some(link) = &self.link {
            match link.send(msg).await {
                Ok(_) => Ok(()),
                Err(e) => {
                    // let msg = e.0;
                    // Clear link so it will reconnect
                    self.link = None;
                    Err(e)
                }
            }
        } else {
            Err(SendError(Message::Error(String::from(
                "No Link established",
            ))))
        }
    }

    async fn process_client_response(&mut self, msg: ClientResponse) {
        println!("PROCESSING CLIENT RESPONSE! {:?}", msg);
        // find callback
        match &msg {
            ClientResponse::Message(msg) => {
                if let Some(cb) = &mut self.on_message {
                    cb(&self.client_channel, msg.clone());
                }
            }
            ClientResponse::Connected => {
                if let Some(cb) = &mut self.on_connect {
                    cb(&self.client_channel);
                }
            }
            ClientResponse::Disconnected => {
                if let Some(cb) = &mut self.on_disconnect {
                    cb(&self.client_channel);
                }
            }
            ClientResponse::Terminated(builder) => {
                if let Some(cb) = &mut self.on_terminate {
                    cb(builder.clone());
                }
            }
            ClientResponse::Denied(builder) => {
                if let Some(cb) = &mut self.on_deny {
                    cb(builder.clone());
                }
            }
        }

        // send through channels
        self.channels.retain(|ch| match ch.send(msg.clone()) {
            Ok(_) => true,
            Err(_) => false,
        });
    }

    async fn connect(&mut self) -> Option<String>{
        // Try each connection method in turn

        // Last known address
        if self.state.last_addr_enable {
            if let Some(addr) = &self.state.last_addr_local {
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");
                if let Some(link) = Link::connect(self_relation, addr, host_relation).await {
                    self.link = Some(link);
                    return Some(addr.clone());
                }
            }
            if let Some(addr) = &self.state.last_addr_global {
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");
                if let Some(link) = Link::connect(self_relation, addr, host_relation).await {
                    self.link = Some(link);
                    return Some(addr.clone());
                }
            }
        }

        // Beacon
        if self.state.beacon_enable {
            println!("using beacon...");
            if let Some(addr) = beacon_lookout_one().await {
                println!("found beacon addr {:?}", addr);
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");
                if let Some(link) = Link::connect(self_relation, addr.clone(), host_relation).await {
                    println!("established beacon link");
                    self.link = Some(link);
                    return Some(addr);
                }else{
                    println!("failed to connect using beacon");
                }
            }
        }

        // Chord
        if self.state.chord_enable {
            for addr in &self.state.chord_addrs {
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");

                let mut assoc: AssociateClient<String, SpiderId2048> =
                    TCPAdaptor::associate_client(addr.to_string());
                assoc
                    .send_op(dht_chord::associate::AssociateRequest::GetAdvertOf {
                        id: host_relation.id.clone(),
                    })
                    .await;
                let addr = match timeout(Duration::from_secs(10), assoc.recv_op()).await {
                    Ok(Some(dht_chord::associate::AssociateResponse::AdvertOf {
                        data, ..
                    })) => match data {
                        Some(data) => match String::from_utf8(data) {
                            Ok(addr) => addr,
                            Err(_) => continue,
                        },
                        None => continue,
                    },
                    _ => {
                        continue;
                    }
                };

                if let Some(link) = Link::connect(self_relation, addr.clone(), host_relation).await {
                    self.link = Some(link);
                    return Some(addr);
                }
            }
        }

        // Fixed address
        if self.state.fixed_addr_enable {
            for addr in &self.state.fixed_addrs {
                let self_relation = self.state.self_relation.clone();
                let host_relation = self
                    .state
                    .host_relation
                    .clone()
                    .expect("Host relation should always be set if connected");
                if let Some(link) = Link::connect(self_relation, addr, host_relation).await {
                    self.link = Some(link);
                    return Some(addr.clone());
                }
            }
        }

        // If they all fail sleep for 15s
        sleep(Duration::from_secs(15)).await;
        None
    }
}

use std::collections::BTreeMap;

use spider_link::{message::Message, Link, Relation, Role, SpiderId2048};
use tokio::{
    sync::mpsc::{channel, error::SendError, Receiver, Sender},
    task::{JoinError, JoinHandle},
};

use crate::{config::SpiderConfig, state_data::StateData};

use super::{message::ProcessorMessage, sender::ProcessorSender};

mod message;
pub use message::RouterProcessorMessage;

pub(crate) struct RouterProcessor {
    sender: Sender<RouterProcessorMessage>,
    handle: JoinHandle<()>,
}

impl RouterProcessor {
    pub fn new(config: SpiderConfig, state: StateData, sender: ProcessorSender) -> Self {
        let (router_sender, router_receiver) = channel(50);
        let processor = RouterProcessorState::new(config, state, sender, router_receiver);
        let handle = processor.start();
        Self {
            sender: router_sender,
            handle,
        }
    }

    pub async fn send(
        &mut self,
        message: RouterProcessorMessage,
    ) -> Result<(), SendError<RouterProcessorMessage>> {
        self.sender.send(message).await
    }

    pub async fn join(self) -> Result<(), JoinError> {
        self.handle.await
    }
}

pub(crate) struct RouterProcessorState {
    config: SpiderConfig,
    state: StateData,
    sender: ProcessorSender,
    receiver: Receiver<RouterProcessorMessage>,

    peer_links: BTreeMap<SpiderId2048, Link>,
    peripheral_links: BTreeMap<SpiderId2048, Link>,
}

impl RouterProcessorState {
    pub fn new(
        config: SpiderConfig,
        state: StateData,
        sender: ProcessorSender,
        receiver: Receiver<RouterProcessorMessage>,
    ) -> Self {
        Self {
            config,
            state,
            sender,
            receiver,

            peer_links: BTreeMap::new(),
            peripheral_links: BTreeMap::new(),
        }
    }

    fn start(mut self) -> JoinHandle<()> {
        let handle = tokio::spawn(async move {
            loop {
                let msg = match self.receiver.recv().await {
                    Some(msg) => msg,
                    None => break,
                };

                match msg {
                    RouterProcessorMessage::NewLink(link) => self.new_link(link),
                    RouterProcessorMessage::SendMessage(rel, msg) => self.send_msg(rel, msg).await,
                    RouterProcessorMessage::MulticastMessage(rels, msg) => {
                        self.multicast_msg(rels, msg).await
                    }
                    RouterProcessorMessage::RouteEvent(event) => todo!(),
                    RouterProcessorMessage::Upkeep => {}
                }
            }
        });
        handle
    }

    fn new_link(&mut self, mut link: Link) {
        // get reciever+relation from link
        let mut rx = match link.take_recv() {
            Some(rx) => rx,
            None => return,
        };
        let relation = link.other_relation().clone();

        // add link to structures
        let id = link.other_relation().id.clone();
        match link.other_relation().role {
            spider_link::Role::Peer => {
                self.peer_links.insert(id, link);
            }
            spider_link::Role::Peripheral => {
                self.peripheral_links.insert(id, link);
            }
        }

        // start link processor
        let channel = self.sender.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Some(msg) => {
                        match channel
                            .send(ProcessorMessage::RemoteMessage(relation.clone(), msg))
                            .await
                        {
                            Ok(_) => {}
                            Err(_) => break,
                        };
                    }
                    None => break, // connection is finished
                }
            }
        });
    }

    async fn send_msg(&mut self, relation: Relation, msg: Message) {
        println!("Sending message: {:?}", msg);
        let link = match relation.role {
            Role::Peer => self.peer_links.get_mut(&relation.id),
            Role::Peripheral => self.peripheral_links.get_mut(&relation.id),
        };
        match link {
            Some(link) => {
                link.send(msg).await;
            }
            None => {} // no link, no send at the moment (should buffer messages and start a new connection)
        }
    }

    async fn multicast_msg(&mut self, relations: Vec<Relation>, msg: Message) {
        for relation in relations {
            self.send_msg(relation, msg.clone()).await
        }
    }
}

// async fn find_public_address(state_data: &StateData) -> String{
// 	// query chord
// 	for (peer_addr, _) in state_data.chord_lru.iter(){
// 		let peer = TCPAdaptor::associate_client(peer_addr);
// 		// peer.
// 	}
// 	// query public ip package
// 	if let Some(ip) = public_ip::addr().await {
// 		return ip.to_string();
// 	} else {
// 		panic!("couldn't get an IP address");
// 	}
// }

// async fn start_chord(config: &SpiderConfig, state_data: &StateData) -> ChordHandle<String, SpiderId2048>{
// 	let mut chord = TCPChord::new(config.listen_addr.clone() , state_data.id);
// 	chord.set_advert(Some(config.pub_addr.as_bytes().to_vec()));
// 	let handle = chord.start(None).await;
// }

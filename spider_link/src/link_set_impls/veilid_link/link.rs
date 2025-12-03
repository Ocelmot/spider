use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::mpsc::{channel, Receiver, Sender};
use tracing::info;

use crate::error::ErrorKind;
use crate::{
    error::ProblemWrap,
    link::{protocol::LinkProtocol, Link},
    LinkResult,
};
use crate::{Relation, SelfRelation};

/// Represents a [Link] between two members of the spider network that is sent
/// via the Veilid network.
pub struct VeilidLink {
    self_relation: Arc<SelfRelation>,
    relation: Arc<Relation>,
    tx: Sender<Vec<u8>>,
    rx: Option<Receiver<Vec<u8>>>,
}

impl VeilidLink {
    pub(crate) fn new(
        self_rel: SelfRelation,
        rel: Relation,
        tx: Sender<Vec<u8>>,
        rx: Receiver<Vec<u8>>,
    ) -> Self {
        Self {
            self_relation: Arc::new(self_rel),
            relation: Arc::new(rel),
            tx,
            rx: Some(rx),
        }
    }
}

impl Link for VeilidLink {
    fn self_relation(&self) -> &SelfRelation {
        &self.self_relation
    }

    fn other_relation(&self) -> &Relation {
        &self.relation
    }

    async fn send(&mut self, msg: LinkProtocol) -> LinkResult {
        let data = msg.serialize();
        self.tx.send(data).await.wrap()
    }

    async fn recv(&mut self) -> LinkResult<LinkProtocol> {
        let rx = self.rx.as_mut().wrap_problem(ErrorKind::Taken)?;
        let data = rx.recv().await.wrap()?;
        LinkProtocol::deserialize(&mut VecDeque::from(data))
    }

    fn take_reader(&mut self) -> LinkResult<Receiver<LinkProtocol>> {
        let mut rx = self.rx.take().wrap()?;
        let (mapped_tx, mapped_rx) = channel(25);
        tokio::spawn(async move {
            loop {
                let bytes = rx.recv().await.wrap()?;
                let mut bytes = VecDeque::from(bytes);
                let Ok(link_protocol) = LinkProtocol::deserialize(&mut bytes) else {
                    info!("Failed to deserialize bytes from inner hub");
                    continue;
                };
                mapped_tx.send(link_protocol).await.wrap()?;
            }
            #[allow(unreachable_code)]
            LinkResult::Ok(())
        });
        Ok(mapped_rx)
    }

    fn max_size(&self) -> u32 {
        20000
    }

    fn is_closed(&mut self) -> bool {
        self.tx.is_closed()
    }
}

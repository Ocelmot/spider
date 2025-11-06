use std::{collections::HashMap, time::Duration, u32};

use rand::{distributions::Bernoulli, prelude::Distribution, thread_rng};
use tokio::{
    select,
    sync::mpsc::{channel, Receiver, Sender},
    time::{sleep_until, Instant},
};
use tracing::debug;

use crate::{
    error::{ErrorKind, ProblemWrap}, link::{link_set::Deadline, protocol::LinkProtocol, Link}, LinkError, LinkResult, Relation, SelfRelation
};

/// A PipeLinkBuilder sets the configuration for a pair of [PipeLink]s.
///
/// This allows a pair of connected PipeLinks to be created with additional
/// settings that may be useful for testing. E.g. the link could expire and
/// close after a certain period of time.
pub struct PipeLinkBuilder {
    max_size: u32,
    reliability: f64,
    expiration: Option<Duration>,
    latency: Option<Duration>,
}

impl PipeLinkBuilder {
    /// Create a new PipeLinkBuilder with the default characteristics
    pub fn new() -> Self {
        Self {
            max_size: u32::MAX,
            reliability: 1.0,
            expiration: None,
            latency: None,
        }
    }

    /// Constrains the max size of a slice that can be sent through the PipeLink
    pub fn max_size(mut self, max_size: u32) -> Self {
        self.max_size = max_size;
        self
    }

    /// Causes the PipeLink to send items sent through it with the given
    /// probability. Probability is [0.0, 1.0], 1.0 indicates the message will
    /// always be sent.
    pub fn reliability(mut self, reliability: f64) -> Self {
        self.reliability = reliability;
        self
    }

    /// Causes the PipeLink to expire after a certain period of time.
    pub fn expiration(mut self, expiration: Option<Duration>) -> Self {
        self.expiration = expiration;
        self
    }

    /// Causes the PipeLink to add the given latency to each sent message
    pub fn latency(mut self, latency: Option<Duration>) -> Self {
        self.latency = latency;
        self
    }

    /// Create a linked pair of PipeLinks with the builder's current
    /// configuration.
    pub fn create_pair(&self, rel1: SelfRelation, rel2: SelfRelation) -> (PipeLink, PipeLink) {
        let (tx1, rx1) = channel(1000);
        let (tx2, rx2) = channel(1000);

        let expiration = if let Some(expiration) = self.expiration {
            Deadline::new_deadline(Instant::now() + expiration)
        } else {
            Deadline::new()
        };

        let first = PipeLink {
            id: rand::random(),
            self_rel: rel1.clone(),
            other_rel: rel2.relation.clone(),
            tx: tx1,
            rx: Some(rx2),
            peeked: None,
            max_size: self.max_size,
            expiration,
            latency: self.latency.clone(),
            reliability: Bernoulli::new(1.0).unwrap(),
        };

        let expiration = if let Some(expiration) = self.expiration {
            Deadline::new_deadline(Instant::now() + expiration)
        } else {
            Deadline::new()
        };

        let second = PipeLink {
            id: rand::random(),
            self_rel: rel2,
            other_rel: rel1.relation,
            tx: tx2,
            rx: Some(rx1),
            peeked: None,
            max_size: self.max_size,
            expiration,
            latency: self.latency,
            reliability: Bernoulli::new(1.0).unwrap(),
        };

        (first, second)
    }
}

/// Pipe link is an implementation of [Link] backed by a pair of mpsc channels.
/// This is primarily useful for testing the rest of the implementation.
pub struct PipeLink {
    id: u32,
    self_rel: SelfRelation,
    other_rel: Relation,
    tx: Sender<(LinkProtocol, Instant)>,
    rx: Option<Receiver<(LinkProtocol, Instant)>>,
    peeked: Option<(LinkProtocol, Instant)>,
    max_size: u32,
    reliability: Bernoulli,
    expiration: Deadline,
    latency: Option<Duration>,
}

impl PipeLink {
    /// Create a pair of PipeLinks that are linked to each other.
    pub fn create_pair(rel1: SelfRelation, rel2: SelfRelation) -> (PipeLink, PipeLink) {
        let (tx1, rx1) = channel(1000);
        let (tx2, rx2) = channel(1000);

        let first = PipeLink {
            id: rand::random(),
            self_rel: rel1.clone(),
            other_rel: rel2.relation.clone(),
            tx: tx1,
            rx: Some(rx2),
            peeked: None,
            max_size: u32::MAX,
            expiration: Deadline::new(),
            latency: None,
            reliability: Bernoulli::new(1.0).unwrap(),
        };

        let second = PipeLink {
            id: rand::random(),
            self_rel: rel2,
            other_rel: rel1.relation,
            tx: tx2,
            rx: Some(rx1),
            peeked: None,
            max_size: u32::MAX,
            expiration: Deadline::new(),
            latency: None,
            reliability: Bernoulli::new(1.0).unwrap(),
        };

        (first, second)
    }

    /// Get the randomly assigned id for this pipe end
    pub fn id(&self) -> u32 {
        self.id
    }


}

impl Link for PipeLink {
    fn self_relation(&self) -> &SelfRelation {
        &self.self_rel
    }

    fn other_relation(&self) -> &Relation {
        &self.other_rel
    }

    async fn send(&mut self, msg: LinkProtocol) -> LinkResult {
        let reliability_success = {
            let mut rng = thread_rng();
            self.reliability.sample(&mut rng)
        };

        
        if reliability_success {
            debug!("Pipe {}: send msg {:?}", self.id, msg);
            self.tx.send((msg, Instant::now())).await.wrap()?;
        } else{
            debug!("Pipe {}: send (dropped) msg {:?}", self.id, msg);
        }
        Ok(())
    }

    async fn recv(&mut self) -> LinkResult<LinkProtocol> {
        if self.rx.is_none() {
            return Err(LinkError::new().problem(ErrorKind::Taken));
        }
        if let Some((msg, timestamp)) = &self.peeked {
            if let Some(latency) = self.latency {
                sleep_until(*timestamp + latency).await;
            }
            let msg = msg.clone();
            self.peeked = None;
            debug!("Pipe {}: read msg {:?}", self.id, msg);
            return Ok(msg);
        }
        select! {
            msg = self.rx.as_mut().unwrap().recv() => {
                let (msg, timestamp) = msg.wrap()?;
                self.peeked = Some((msg.clone(), timestamp));
                if let Some(latency) = self.latency {
                    sleep_until(timestamp + latency).await;
                }
                self.peeked = None;
                debug!("Pipe {}: read msg {:?}", self.id, msg);
                return Ok(msg);
            },
            _ = &mut self.expiration, if self.expiration.has_deadline() => {
                debug!("Pipe {}: expired", self.id);
                self.rx.as_mut().unwrap().close();
                return Err(LinkError::new().problem(ErrorKind::Closed))
            }
        }
    }

    fn take_reader(&mut self) -> LinkResult<Receiver<LinkProtocol>> {
        let peeked = self.peeked.take();
        let mut rx = self.rx.take().wrap_problem(ErrorKind::Taken)?;
        let (mapped_tx, mapped_rx) = channel(100);
        let latency = self.latency.clone();
        let id = self.id;
        let map_task = tokio::spawn(async move {
            if let Some((msg, timeout)) = peeked {
                if let Some(latency) = latency {
                    sleep_until(timeout + latency).await;
                }
                debug!("Pipe {}: read msg {:?}", id, msg);
                mapped_tx.send(msg).await.wrap()?;
            }
            loop {
                if let Some((msg, timeout)) = rx.recv().await {
                    if let Some(latency) = latency {
                        sleep_until(timeout + latency).await;
                    }
                    debug!("Pipe {}: read msg {:?}", id, msg);
                    mapped_tx.send(msg).await.wrap()?;
                } else {
                    break; // connection was closed
                }
            }
            LinkResult::Ok(())
        });

        // if there is an expiration set, cancel the first task
        let mut expiration = self.expiration.clone();
        let id = self.id;
        if expiration.has_deadline() {
            tokio::spawn(async move {
                expiration.await;
                map_task.abort();
                debug!("Pipe {}: expired", id);
            });
        }
        Ok(mapped_rx)
    }

    fn max_size(&self) -> u32 {
        self.max_size
    }

    fn is_closed(&mut self) -> bool {
        self.tx.is_closed()
    }
}

/// Serves to simulate the listen/connect actions when establishing a connection
/// between [PipeLink]s.
/// 
/// This can be used to set up a fake network for testing or simulation
/// purposes.
pub struct PipeLinkHub {
    link_builder: PipeLinkBuilder,
    listeners: HashMap<String, (SelfRelation, Sender<PipeLink>)>,
}

impl PipeLinkHub {
    /// Create a new PipeLinkHub.
    /// 
    /// The connections it returns will be generated with the given
    /// [PipeLinkBuilder] which can be given options to make the links timeout,
    /// drop messages, etc.
    pub fn new(link_builder: PipeLinkBuilder) -> Self {
        Self {
            link_builder,
            listeners: HashMap::new(),
        }
    }

    /// Returns a [Receiver<PipeLink>] that will "listen" at the given address.
    /// Connections to that address will cause a new PipeLink to be created and
    /// sent through the receiver.
    pub fn listen(&mut self, self_rel: SelfRelation, addr: String) -> Receiver<PipeLink> {
        let (tx, rx) = channel(10);
        self.listeners.insert(addr, (self_rel, tx));
        rx
    }

    /// Creates a connection to a listening address if there is one.
    pub async fn connect(&mut self, self_rel: SelfRelation, dest_addr: &String) -> Option<PipeLink> {
        match self.listeners.get(dest_addr) {
            Some((listen_rel, listener)) => {
                let (link1, link2) = self.link_builder.create_pair(self_rel, listen_rel.clone());
                match listener.send(link2).await {
                    Ok(_) => Some(link1),
                    Err(_) => {
                        self.listeners.remove(dest_addr);
                        None
                    }
                }
            }
            None => None,
        }
    }
}

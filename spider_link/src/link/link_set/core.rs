use std::{
    cmp::Ordering,
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures::{stream::SelectAll, StreamExt};
use rand::{prelude::Distribution, random, thread_rng};
use tokio::{
    select,
    sync::mpsc::{channel, Receiver, Sender},
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{Instrument, debug, info, instrument, trace, trace_span};

use crate::{
    error::{ErrorKind, ProblemWrap},
    link::protocol::LinkProtocol,
    LinkError, LinkResult, Relation, SelfRelation,
};

use super::{
    deadline::Deadline, instrument_link::InstrumentLink, slice_manager::SliceManager,
    state::CoreState, LinkConnector, LinkSetControl, LinkSetMsg,
};

/// The duration to wait before resending a message slice
const RESEND: Duration = Duration::from_secs(5);
/// The frequency at which to run the upkeep function
const UPKEEP: Duration = Duration::from_secs(15);
/// The duration after the last connection is removed to attempt a reconnection
/// without fully disconnecting.
const TRANSIENT: Duration = Duration::from_secs(5);
/// If a connection is not established within this limit, the link disconnects again.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
/// Resets with the same epoch within this duration will not trigger a reset
const RESET_DEBOUNCE: Duration =
    Duration::saturating_add(Duration::saturating_mul(RESEND, 2), Duration::from_secs(1));
/// If the latency for a connection is above this amount it considers the
/// connection disconnected.
const MAX_LATENCY: Duration = Duration::from_millis(3000);

pub struct LinkSetCore {
    self_rel: SelfRelation,
    rel: Relation,

    rx: Receiver<LinkSetControl>,
    tx: Sender<LinkSetMsg>,

    ctrl_tx: Sender<LinkSetControl>,
    state: CoreState,
    epoch: u64,
    resetting: bool,
    reset_debounce: Instant,

    auto_reconnect: bool,
    grace_period: Option<u64>,
    addrs: Vec<String>,
    connectors: Vec<Arc<LinkConnector>>,

    links: Vec<InstrumentLink>,
    /// Used to refer to a sender
    next_link_index: u64,
    /// maps outgoing slices' seq numbers to a slice manager
    outgoing_slices: BTreeMap<u64, SliceManager>,
    /// messages that are to be sent when we get a new epoch
    pending_slices: Vec<SliceManager>,
    /// The next sequence number to assign to an incoming message
    next_seq: u64,

    readers: SelectAll<ReceiverStream<(u64, LinkProtocol)>>,
    /// Maps incoming slices' seq numbers to a slice manager
    incoming_slices: BTreeMap<u64, SliceManager>,
    /// The seq_num and last_index of the last ack that this node sent.
    last_ack: (u64, u64),

    resend: Deadline,
    upkeep: Deadline,
}

impl LinkSetCore {
    pub(super) fn start(
        self_rel: SelfRelation,
        rel: Relation,
    ) -> (Sender<LinkSetControl>, Receiver<LinkSetMsg>) {
        trace!("LinkSetCore starting");
        let (to_core, rx) = channel(10);
        let (tx, from_core) = channel(10);

        let mut core = Self {
            self_rel,
            rel,

            rx,
            tx,

            ctrl_tx: to_core.clone(),
            state: CoreState::Disconnected,
            epoch: 0,
            resetting: true,
            reset_debounce: Instant::now(),

            auto_reconnect: false,
            grace_period: None,
            addrs: Vec::new(),
            connectors: Vec::new(),

            links: Vec::new(),
            next_link_index: 0,
            outgoing_slices: BTreeMap::new(),
            pending_slices: Vec::new(),
            next_seq: 1,

            readers: SelectAll::new(),
            incoming_slices: BTreeMap::new(),
            last_ack: (0, 0),

            resend: Deadline::new(),
            upkeep: Deadline::new_repeat(UPKEEP),
        };
        let span = trace_span!("LinkSet", ID = random::<u16>());
        tokio::spawn(async move {
            loop {
                core.process().await?;
            }
            #[allow(unreachable_code)]
            LinkResult::Ok(())
        }.instrument(span));

        (to_core, from_core)
    }

    async fn process(&mut self) -> LinkResult {
        trace!("LinkSetCore processing");
        trace!("Readers: {} Links: {}", self.readers.len(), self.links.len());
        select! {
            ctrl_msg = self.rx.recv() => {
                self.process_control(ctrl_msg).await?;
            },
            link_msg = self.readers.next(), if !self.readers.is_empty() => {
                if let Some((id, msg)) = link_msg {
                    self.process_link_msg(id, msg).await?;
                }else{
                    info!("readers got None");
                }
            },
            _ = &mut self.resend, if self.resend.has_deadline() => {
                self.process_resend().await?;
            }
            _ = &mut self.upkeep, if self.upkeep.has_deadline() => {
                self.process_upkeep().await?;
            }
        }
        Ok(())
    }

    async fn process_control(&mut self, msg: Option<LinkSetControl>) -> LinkResult {
        let msg = msg.ok_or(LinkError::from(ErrorKind::Closed))?;

        match msg {
            LinkSetControl::Connect => {
                trace!("LinkSetControl::Connect, starting connection");
                let began_connecting = self.state.connect(
                    &self.self_rel,
                    &self.rel,
                    &self.ctrl_tx,
                    &self.addrs,
                    &self.connectors,
                );
                if began_connecting {
                    self.tx.send(LinkSetMsg::Connecting(true)).await.wrap()?;
                    self.resetting = true;
                    trace!("Began connecting with auto_reconnect={}", self.auto_reconnect);
                    if !self.auto_reconnect {
                        self.upkeep.set_deadline_from_now(CONNECTION_TIMEOUT);
                    }
                }
                Ok(())
            }
            LinkSetControl::Disconnect => self.disconnect().await,
            LinkSetControl::AddConnector(conn) => {
                trace!("LinkSetCore adding connector");
                let conn = Arc::new(conn);
                self.state.add_connector(&conn).await;
                self.connectors.push(conn);
                Ok(())
            }
            LinkSetControl::Reconnect(auto_reconnect) => {
                self.auto_reconnect = auto_reconnect;
                Ok(())
            }
            LinkSetControl::AllowReconnect(grace_period) => {
                self.grace_period = grace_period;
                Ok(())
            }
            LinkSetControl::AddAddress(addr) => {
                trace!("LinkSetCore adding addr {}", addr);
                self.state.add_addr(&addr).await;
                self.addrs.push(addr);
                Ok(())
            }
            LinkSetControl::TryAddress(addr) => {
                trace!("LinkSetCore trying addr {}", addr);
                self.state.try_addr(&addr).await;
                Ok(())
            }
            LinkSetControl::AddLink(link) => {
                trace!("LinkSetCore adding link");
                if *link.self_relation() != self.self_rel || *link.other_relation() != self.rel {
                    info!("Passed a link to a link set with mismatched relationships");
                    return Ok(());
                }

                let mut instrumented = InstrumentLink::new(link, self.next_link_index);
                self.next_link_index += 1;

                let read_stream = instrumented.take_reader().wrap_problem(ErrorKind::Taken)?;
                self.readers.push(read_stream);

                self.links.insert(0, instrumented);

                trace!("LinkSetControl::AddLink, transitioning to connected");
                if self.state.has_connector() {
                    // if the state had a connector, it is now no longer connecting
                    self.tx.send(LinkSetMsg::Connecting(false)).await.wrap()?;
                }
                if self.state.connected() {
                    trace!("Became connected");
                    
                    self.upkeep.set_repeat(UPKEEP);
                    self.send_reset(true).await;
                    self.resend.set_deadline_from_now(RESEND);
                    self.resetting = true;
                }
                Ok(())
            }
            LinkSetControl::Message(message, epoch) => {
                // if the message's epoch does not match the current epoch, skip
                if let Some(epoch) = epoch {
                    if epoch != self.epoch {
                        return Ok(());
                    }
                }

                debug!("sending message: {:?}", message);
                debug!("link state: {:?}", self.state);
                // Serialize the message and store it in the link set
                let seq = self.next_seq;
                self.next_seq += 1;
                let data = serde_json::to_vec(&message).wrap()?;
                let mgr = SliceManager::from_vec(self.epoch, seq, data);

                // if the core is not connected or is connected and resetting,
                // and the message's epoch is none, add message to pending
                // connection messages. Else, add to outgoing slices
                if !self.state.is_connected() || self.resetting {
                    if epoch.is_none() {
                        self.pending_slices.push(mgr);
                    }
                }else{
                    self.outgoing_slices.insert(seq, mgr);
                }

                // if connected, send
                if let CoreState::Connected = self.state {
                    // send fragments
                    if !self.resetting {
                        self.send_slices(seq).await;
                    }

                    // set deadline if not already set
                    if !self.resend.has_deadline() {
                        self.resend.set_deadline_from_now(RESEND);
                    }
                }

                if let CoreState::Disconnected = self.state {
                    self.state.connect(
                        &self.self_rel,
                        &self.rel,
                        &self.ctrl_tx,
                        &self.addrs,
                        &self.connectors,
                    );
                    self.tx.send(LinkSetMsg::Connecting(true)).await.wrap()?;
                    self.upkeep.clear();
                    trace!("Began connecting with auto_reconnect={}", self.auto_reconnect);
                    if !self.auto_reconnect {
                        self.upkeep.set_deadline_from_now(CONNECTION_TIMEOUT);
                    }
                }

                Ok(())
            }
        }
    }

    async fn process_link_msg(&mut self, id: u64, msg: LinkProtocol) -> LinkResult {
        match &msg {
            LinkProtocol::Reset { epoch, request } => {
                if self.resetting {
                    trace!("Is resetting");
                    if *request {
                        // update epoch/reply
                        match wrapping_cmp(self.epoch, *epoch) {
                            Ordering::Greater => {
                                // send our own reset/epoch (not reply)
                                self.send_reset(true).await;
                            }
                            Ordering::Equal => {
                                // on the same page, stop resetting. Reply with our own reset/epoch
                                self.send_reset(false).await;
                                // clear outgoing messages, insert pending messages
                                self.outgoing_slices.clear();
                                for mut msg in self.pending_slices.drain(..){
                                    msg.set_epoch(self.epoch);
                                    msg.set_seq(self.next_seq);
                                    self.outgoing_slices.insert(self.next_seq, msg);
                                    self.next_seq += 1;
                                }
                                // send all pending messages
                                self.send_all_slices().await;
                                self.resend.set_deadline_from_now(RESEND);

                                // remove resetting
                                self.resetting = false;
                                self.reset_debounce = Instant::now();

                                // send connected message
                                trace!("Sending connected");
                                self.tx
                                    .send(LinkSetMsg::Connected(self.epoch))
                                    .await
                                    .wrap()?;
                            }
                            Ordering::Less => {
                                // update our epoch to match
                                // reply with updated epoch
                                self.epoch = *epoch;
                                self.send_reset(true).await;
                            }
                        }
                    } else {
                        trace!("responding to reply");
                        // check for confirmation of epoch, then start
                        if *epoch == self.epoch {
                            trace!("epoch match");
                            // clear outgoing messages, insert pending messages
                            self.outgoing_slices.clear();
                            for mut msg in self.pending_slices.drain(..){
                                msg.set_epoch(self.epoch);
                                msg.set_seq(self.next_seq);
                                self.outgoing_slices.insert(self.next_seq, msg);
                                self.next_seq += 1;
                            }

                            // send all outgoing messages
                            self.send_all_slices().await;
                            self.resend.set_deadline_from_now(RESEND);

                            // remove resetting
                            self.resetting = false;
                            self.reset_debounce = Instant::now();

                            // send connected message
                            trace!("Sending connected");
                            self.tx
                                .send(LinkSetMsg::Connected(self.epoch))
                                .await
                                .wrap()?;
                        } else {
                            trace!("epoch mismatch");
                            // send our own reset/epoch (not reply)
                            self.send_reset(true).await;
                        }
                    }
                } else {
                    trace!("reset outside of resetting");
                    if *request {
                        // new request, start resetting
                        if self.reset_debounce.elapsed() > RESET_DEBOUNCE || self.epoch != *epoch {
                            trace!(
                                "debounce elapsed, our epoch {}, their epoch {}",
                                self.epoch,
                                epoch
                            );
                            self.disconnect().await?;
                            self.resetting = true;
                            self.state.connected();
                            self.epoch = wrapping_max(self.epoch, *epoch);
                            trace!("next epoch {}", self.epoch);
                            self.send_reset(true).await;
                            self.resend.set_deadline_from_now(RESEND);
                        } else {
                            trace!("debounce not elapsed");
                            // Received reset while in the debounce period and
                            // its the same epoch! Resend the reply and reset
                            // the debounce period
                            self.send_reset(false).await;
                            self.reset_debounce = Instant::now();
                        }
                    } else {
                        trace!("reset was reply");
                        // reply outside of resetting, if epoch does not
                        // match,something is de-synced, start resetting.
                        if self.epoch != *epoch {
                            self.disconnect().await?;
                            self.resetting = true;
                            self.state.connected();
                            self.epoch = wrapping_max(self.epoch, *epoch);
                            self.send_reset(true).await;
                            self.resend.set_deadline_from_now(RESEND);
                        }
                    }
                }
            }
            LinkProtocol::Ack {
                epoch,
                seq,
                last_index,
            } => {
                if *epoch != self.epoch {
                    return Ok(()); // discard mismatched epochs
                }
                // add ack to the slice manager if it exists
                if let Some(slice_manager) = self.outgoing_slices.get_mut(&seq) {
                    slice_manager.recv_protocol(&msg);
                    if slice_manager.is_empty() {
                        // all items sent
                        self.outgoing_slices.remove(&seq);
                    }
                }

                // send the ack to the instrumented links for stats
                for link in &mut self.links {
                    link.ack_prev(*seq, *last_index);
                }

                // if there are more messages to receive, set the timeout to
                // resend them.
                if !self.outgoing_slices.is_empty() {
                    self.resend.set_deadline_from_now(RESEND);
                } else {
                    self.resend.clear(); // nothing else to retry sending
                }
            }
            LinkProtocol::MsgSlice {
                epoch,
                seq,
                seq_len,
                ..
            } => {
                if *epoch != self.epoch {
                    return Ok(()); // discard mismatched epochs
                }
                // if we receive a message for which we have already ack'd
                // the ack was probably lost, resend.
                if (*seq, msg.last_index()) <= self.last_ack {
                    self.send_last_ack().await;
                    return Ok(());
                }

                // add received slice
                let mgr = self
                    .incoming_slices
                    .entry(*seq)
                    .or_insert_with(|| SliceManager::new(*epoch, *seq, *seq_len));
                mgr.recv_protocol(&msg);

                // iterate through incoming slices. For each leading full mgr,
                // remove it deserialize the message, and emit
                let mut ack = (0u64, 0u64);
                while let Some(entry) = self.incoming_slices.first_entry() {
                    if let Some(last_index) = entry.get().last_head_index() {
                        ack = (*entry.key(), last_index);
                    };
                    if let Some(data) = entry.get().data() {
                        // deserialize, add to outgoing queue
                        match serde_json::from_slice(data) {
                            Ok(msg) => {
                                self.tx
                                    .send(LinkSetMsg::Message(msg, self.epoch))
                                    .await
                                    .wrap()?;
                            }
                            Err(_) => {}
                        }

                        entry.remove();
                    } else {
                        break;
                    }
                }

                // reply with ack if appropriate
                if ack > self.last_ack {
                    self.send_ack(ack.0, ack.1).await;
                }
            },
            LinkProtocol::Ping => {
                // Received a ping, respond on the same link with the pong
                for link in &mut self.links {
                    // a better way to do this would be to have the senders and
                    // receivers in some kind of map, and do a lookup
                    if link.id() == id {
                        let _ = link.send(LinkProtocol::Pong).await;
                        break;
                    }
                }
            },
            LinkProtocol::Pong => {
                // Received a pong, find the link and end the ping
                for link in &mut self.links {
                    if link.id() == id {
                        link.end_ping();
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    async fn process_resend(&mut self) -> LinkResult {
        if self.resetting {
            self.send_reset(true).await;
            self.resend.set_deadline_from_now(RESEND);
            return Ok(());
        }

        if let Some((&seq, _)) = self.outgoing_slices.first_key_value() {
            // resend the first message, starting at the first unacknowledged slice, proceeding through the end.
            self.send_slices(seq).await;
            // Also, need to alert the instrumented links to the lost message.
            for link in &mut self.links {
                link.timeout_seq(seq);
            }
            // reset the deadline
            self.resend.set_deadline_from_now(RESEND);
        } else {
            // there is no outgoing slice waiting for confirmation
            self.resend.clear();
        }

        Ok(())
    }

    async fn disconnect(&mut self) -> LinkResult {
        info!("Link Disconnecting...");
        if self.state.disconnect() {
            trace!("Link Disconnected.");
            // Reset sending items
            self.outgoing_slices.clear();
            self.next_seq = 0;

            // Reset receiving items
            self.incoming_slices.clear();
            self.last_ack = (0, 0);

            // Reset timers
            self.resend.clear();

            // update epoch
            self.epoch += 1;
            self.resetting = false;

            // Inform sender of disconnection
            self.tx.send(LinkSetMsg::Connecting(false)).await.wrap()?;
            self.tx.send(LinkSetMsg::Disconnected).await.wrap()?;
        } else {
            trace!("Already disconnected.");
        }
        Ok(())
    }

    async fn send_reset(&mut self, request: bool) {
        if let Some(link) = choose_link(&mut self.links) {
            let ack = LinkProtocol::Reset {
                epoch: self.epoch,
                request,
            };
            let _ = link.send(ack).await;
        }
    }

    async fn send_ack(&mut self, seq: u64, last_index: u64) {
        let new_ack = (seq, last_index);
        if new_ack > self.last_ack {
            self.last_ack = new_ack;
        }

        if let Some(link) = choose_link(&mut self.links) {
            let ack = LinkProtocol::Ack {
                epoch: self.epoch,
                seq: self.last_ack.0,
                last_index: self.last_ack.1,
            };
            let _ = link.send(ack).await;
        }
    }

    async fn send_last_ack(&mut self) {
        if let Some(link) = choose_link(&mut self.links) {
            let ack = LinkProtocol::Ack {
                epoch: self.epoch,
                seq: self.last_ack.0,
                last_index: self.last_ack.1,
            };
            let _ = link.send(ack).await;
        }
    }

    async fn send_slices(&mut self, seq: u64) {
        let Some(link) = choose_link(&mut self.links) else {
            return;
        };

        if let Some(mgr) = self.outgoing_slices.get(&seq) {
            let size = link.max_size();
            let slices = mgr.get_slices(size);
            for slice in slices {
                let _ = link.send(slice).await;
            }
        }
    }

    async fn send_all_slices(&mut self) {
        trace!("sending all slices");
        let Some(link) = choose_link(&mut self.links) else {
            return;
        };
        debug!("outgoing count {}", self.outgoing_slices.len());
        for mgr in self.outgoing_slices.values() {
            let size = link.max_size();
            let slices = mgr.get_slices(size);
            for slice in slices {
                debug!("sending msg {:?}", slice);
                let _ = link.send(slice).await;
            }
        }
    }

    async fn process_upkeep(&mut self) -> LinkResult {
        trace!("processing upkeep...");
        if let CoreState::GracePeriod = &self.state {
            trace!("Ending grace period.");
            self.disconnect().await?;
            self.upkeep.clear();
            return Ok(());
        }

        // if the state is transient, fall back to connecting
        if let CoreState::Reconnecting(_) = &self.state {
            trace!("ending transient");
            self.state.end_transient();

            // Reset sending items
            self.outgoing_slices.clear();
            self.next_seq = 0;

            // Reset receiving items
            self.incoming_slices.clear();
            self.last_ack = (0, 0);

            // Reset timers
            self.resend.clear();
            self.upkeep.clear();

            // update epoch
            self.epoch += 1;
            self.resetting = false;

            // Inform sender of disconnection
            self.tx.send(LinkSetMsg::Disconnected).await.wrap()?;
            return Ok(());
        }

        // if state is connecting, the connection failed
        if let CoreState::Connecting(_) = &self.state {
            if self.auto_reconnect {
                // auto reconnecting does not time out
                return Ok(());
            }
            trace!("Connecting timed out");
            self.state.disconnect();
            self.upkeep.clear();
            self.tx.send(LinkSetMsg::Connecting(false)).await.wrap()?;
            return Ok(());
        }
        // else if the state is connected, check links, etc.

        // filter out disconnected links
        self.links.retain_mut(|l| {
            if l.is_closed() {
                return false;
            }

            l.end_ping();
            if l.latency() > MAX_LATENCY {
                return false;
            }

            true
        });

        if self.links.is_empty() {
            trace!("No more links");
            self.upkeep.clear();
            // initiate a transient disconnection
            if self.auto_reconnect {
                trace!("Starting transient");
                self.state.start_transient(
                    &self.self_rel,
                    &self.rel,
                    &self.ctrl_tx,
                    &self.addrs,
                    &self.connectors,
                );
                self.tx.send(LinkSetMsg::Connecting(true)).await.wrap()?;
                self.upkeep.set_deadline_from_now(TRANSIENT);
            } else {
                // if there is an auto reconnect grace period, move to reconnecting
                if let Some(grace_period) = self.grace_period {
                    trace!("Entering grace period");
                    self.state.start_grace_period();
                    self.upkeep.clear();
                    self.upkeep
                        .set_deadline_from_now(Duration::from_secs(grace_period));
                } else {
                    trace!("disconnecting");
                    self.disconnect().await?;
                    debug!("link state after disconnect: {:?}", self.state);
                }
            }
            return Ok(());
        }

        // send ping to each link
        for link in &mut self.links{
            let _ = link.send_ping().await;
        }

        // partially sort remaining links
        trace!("ordering links");
        let iter = 0..self.links.len().saturating_sub(1);
        let iter = iter.map(|i| (i, i + 1));
        for (x, y) in iter {
            if let Some(a) = self.links.get(x) {
                trace!("a.latency {:?}", a.latency());
                if let Some(b) = self.links.get(y) {
                    trace!("b.latency {:?}", b.latency());
                    if a.latency() > b.latency() {
                        trace!("swapping links");
                        self.links.swap(x, y);
                    }
                }
            }
        }
        Ok(())
    }
}

fn choose_link(links: &mut Vec<InstrumentLink>) -> Option<&mut InstrumentLink> {
    let skip = rand::distributions::Bernoulli::from_ratio(5, 100).unwrap();
    let mut iter = links.iter_mut();
    let mut link = iter.next();
    while skip.sample(&mut thread_rng()) {
        let next_link = iter.next();
        if let Some(link) = &mut link {
            if link.is_closed() {
                continue;
            }
        }
        if next_link.is_some() {
            link = next_link;
        } else {
            break;
        }
    }
    link
}

/// determines if another u64 is above the first u64, but the comparison wraps
/// at the point opposite from the first.
#[instrument(level = "trace", ret)]
fn wrapping_cmp(this: u64, other: u64) -> Ordering {
    if this == other {
        return Ordering::Equal;
    }

    let other = other.wrapping_sub(this);
    if other < u64::MAX / 2 {
        return Ordering::Less;
    } else {
        return Ordering::Greater;
    }
}

/// Returns which is larger, but does the calculation based on the first
/// argument as the starting point (opposite the wrapping point).
#[instrument(level = "trace", ret)]
fn wrapping_max(this: u64, other: u64) -> u64 {
    match wrapping_cmp(this, other) {
        Ordering::Less => other,
        Ordering::Equal => this,
        Ordering::Greater => this,
    }
}

use std::collections::{BTreeMap, VecDeque};

use tokio::{sync::mpsc::channel, time::{Duration, Instant}};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    link::{protocol::LinkProtocol, PinnedLink},
    LinkResult,
};

/// The last N historical samples to keep when calculating latency
static HISTORY_LEN: usize = 6;
static DEFAULT_LATENCY: Duration = Duration::from_millis(100);
/// The amount of time to add to dropped messages to deprioritize unreliable connections.
static DROPPED_COST: Duration = Duration::from_secs(2);

pub(crate) struct InstrumentLink {
    id: u64,
    /// The wrapped link
    link: Box<dyn PinnedLink>,
    /// Maps a seq num to the instant it was sent
    inflight: BTreeMap<(u64, u64), Instant>,
    /// The time the last un-received ping was sent.
    ping: Option<Instant>,
    /// Queue of recent latencies
    recent: VecDeque<Duration>,
}

impl InstrumentLink {
    pub fn new<L: Into<Box<dyn PinnedLink>>>(link: L, id: u64) -> Self {
        let link = link.into();
        let mut recent = VecDeque::with_capacity(HISTORY_LEN + 1);
        for _ in 0..HISTORY_LEN {
            recent.push_back(DEFAULT_LATENCY);
        }

        Self {
            id,
            link,
            inflight: BTreeMap::new(),
            ping: None,
            recent,
        }
    }

    pub fn id(&mut self) -> u64 {
        self.id
    }

    pub async fn send(&mut self, msg: LinkProtocol) -> LinkResult {
        self.inflight
            .insert((msg.seq(), msg.last_index()), Instant::now());
        self.link.send(msg).await
    }

    pub fn ack_prev(&mut self, seq: u64, last_index: u64) {
        let mut prev_elements = self.inflight.split_off(&(seq, last_index + 1));
        std::mem::swap(&mut self.inflight, &mut prev_elements); // split gives the second half not the first

        for sent in prev_elements.values() {
            let latency = sent.elapsed();
            self.add_latency(latency);
        }
    }

    pub fn timeout_seq(&mut self, seq: u64) {
        let mut prev_elements = self.inflight.split_off(&(seq + 1, 0));
        std::mem::swap(&mut self.inflight, &mut prev_elements); // split gives the second half not the first

        for sent in prev_elements.values() {
            let latency = sent.elapsed();
            self.add_latency(latency + DROPPED_COST);
        }
    }

    pub async fn send_ping(&mut self) -> LinkResult {
        // end the previous ping if there was one.
        self.end_ping();
        self.ping = Some(Instant::now());
        self.link.send(LinkProtocol::Ping).await
    }

    pub fn end_ping(&mut self) {
        if let Some(last_ping) = self.ping.take() {
            self.add_latency(last_ping.elapsed());
        }
    }

    pub fn latency(&self) -> Duration {
        if self.recent.len() == 0 {
            // If connection has not been used, assume mediocre latency
            return Duration::from_millis(100);
        }
        self.recent.iter().sum::<Duration>() / self.recent.len() as u32
    }

    pub fn is_closed(&mut self) -> bool {
        self.link.is_closed()
    }

    pub fn max_size(&mut self) -> u32 {
        self.link.max_size()
    }

    fn add_latency(&mut self, latency: Duration) {
        self.recent.push_front(latency);
        self.recent.truncate(HISTORY_LEN);
    }

    #[allow(dead_code)]
    async fn recv(&mut self) -> LinkResult<(u64, LinkProtocol)>{
        let proto = self.link.recv().await?;
        Ok((self.id, proto))
    }

    pub fn take_reader(&mut self) -> LinkResult<ReceiverStream<(u64, LinkProtocol)>> {
        let mut reader = self.link.take_reader()?;
        let (tx, rx) = channel(10);
        let id = self.id;
        tokio::spawn(async move {
            loop {
                let Some(proto) = reader.recv().await else {return};
                if tx.send((id, proto)).await.is_err() {
                    return;
                }
            }
        });
        let x = ReceiverStream::new(rx);
        Ok(x)
    }
}

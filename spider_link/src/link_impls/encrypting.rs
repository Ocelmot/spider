use std::{
    cmp::min,
    collections::VecDeque,
    time::{Duration, Instant},
};

use link_set::links::{Link, LinkReader};
use rand::random;
use sha2::{Digest, Sha256};
use tokio::{select, time::sleep_until};
use tracing::warn;

use crate::{
    crypto_suites::{
        self, by_id,
        ciphers::{Opener, Sealer},
        offer, select_offer, HandshakeRole,
    },
    error::{ErrorKind, Problem, ProblemWrap},
    link_impls::authenticated::Establish,
    LinkError, LinkResult, Relation, SelfRelation,
};

const HANDSHAKE_NONCE_LEN: usize = 16;
const HANDSHAKE_IDENTIFIER: &'static [u8; 8] = &b"SPDRHS01";

const FLOOR_RANK: u8 = 0;
const SEND_INTERVAL: Duration = Duration::from_millis(250);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

const MSG_TAG_DATA: u8 = 0;
const MSG_TAG_F1: u8 = 1;
const MSG_TAG_R: u8 = 2;
const MSG_TAG_F1P: u8 = 3;
const MSG_TAG_F2: u8 = 4;
const MSG_TAG_F3: u8 = 5;
const MSG_TAG_F4: u8 = 6;
const ACK_STR: &'static [u8; 8] = &b"SPDRACKD";

pub(super) struct Encrypting<L: Link> {
    link: L,
    peeked: Option<Vec<u8>>,
    sealer: Box<dyn Sealer>,
    opener: Option<Box<dyn Opener>>,
}

impl<L: Link> Establish for Encrypting<L> {
    type Inner = L;

    async fn listen(self_relation: SelfRelation, mut link: L) -> LinkResult<(Relation, Self)> {
        let timeout = Instant::now() + HANDSHAKE_TIMEOUT;
        let role = HandshakeRole::Responder;
        let mut transcript = Sha256::new();

        // F1
        // Recv and parse
        let f1 = send_recv(&mut link, None, None, &[MSG_TAG_F1], timeout).await?;
        let (_tag, remainder) = f1.split_at_checked(1).ok_or(ErrorKind::Deserialization)?;
        let (hs_id, remainder) = remainder
            .split_at_checked(HANDSHAKE_IDENTIFIER.len())
            .ok_or(ErrorKind::Deserialization)?;
        let (_nonce_i, remainder) = remainder
            .split_at_checked(HANDSHAKE_NONCE_LEN)
            .ok_or(ErrorKind::Deserialization)?;
        let (byte, remainder) = remainder
            .split_at_checked(1)
            .ok_or(ErrorKind::Deserialization)?;
        let offer_len = byte[0];
        let (offers, mut kex_i) = remainder
            .split_at_checked(offer_len as usize)
            .ok_or(ErrorKind::Deserialization)?;
        transcript.update(&f1);

        // Check
        if hs_id != HANDSHAKE_IDENTIFIER {
            Err(ErrorKind::Authentication).msg("Invalid identifier")?;
        }
        if offer_len == 0 {
            Err(ErrorKind::Authentication).msg("No suites offered")?;
        }
        let sent_offer_id = offers[0];
        let selected_suite = select_offer(offers, FLOOR_RANK).ok_or(ErrorKind::Authentication)?;

        // Renegotiate suite path
        let mut _f1po = None;
        let mut f1_tag = MSG_TAG_F1;
        if selected_suite.id() != sent_offer_id {
            // Initiate retry path with selected offer to get recalculated kex_i
            // R
            let r = vec![MSG_TAG_R, selected_suite.id()];
            transcript.update(&r);

            // R -> F1'
            let f1p = send_recv(
                &mut link,
                Some(&r),
                Some(MSG_TAG_F1),
                &[MSG_TAG_F1P],
                timeout,
            )
            .await?;
            transcript.update(&f1p);

            _f1po = Some(f1p);
            f1_tag = MSG_TAG_F1P;
            kex_i = &_f1po.as_ref().unwrap()[1..]; // strip tag
        }

        let handshake = selected_suite.start(role);

        // F2
        let nonce_r: u128 = rand::random();
        let kex_r = handshake.local_kex();

        let mut f2 = Vec::new();
        f2.push(MSG_TAG_F2);
        f2.extend(nonce_r.to_be_bytes());
        f2.extend((kex_r.len() as u32).to_be_bytes());
        f2.extend(kex_r);

        transcript.update(&f2);

        let mut our_hash = transcript.clone();
        our_hash.update(role.tag());
        let sig = self_relation.sign_digest(our_hash);
        f2.extend(sig);

        // F2 -> F3
        let f3 = send_recv(&mut link, Some(&f2), Some(f1_tag), &[MSG_TAG_F3], timeout).await?;

        let (mut sealer, mut opener) = handshake.finish(kex_i)?;
        // F3
        let opened = opener.open(&f3[1..])?; // Strip tag
        let (rel_len, remainder) = opened.split_at_checked(4).wrap_msg("F3 too short")?;
        let rel_len = u32::from_be_bytes(rel_len.try_into().unwrap());
        let (rel_bytes, sig) = remainder
            .split_at_checked(rel_len as usize)
            .wrap_msg("F3 missing relation")?;
        let other_relation = Relation::deserialize(&mut VecDeque::from(rel_bytes.to_vec()))?;

        let mut their_hash = transcript.clone();
        their_hash.update(role.other().tag());
        if !other_relation.verify_digest(their_hash, sig) {
            Err(ErrorKind::Authentication).msg("Signature failed")?
        }

        // F4 -> data/timeout
        let mut f4 = Vec::new();
        f4.push(MSG_TAG_F4);
        f4.extend_from_slice(&sealer.seal(ACK_STR));
        let timeout = min(timeout, Instant::now() + (4 * SEND_INTERVAL));
        let mut data = send_recv(
            &mut link,
            Some(&f4),
            Some(MSG_TAG_F3),
            &[MSG_TAG_DATA],
            timeout,
        )
        .await
        .map(|data| Some(data));

        // map Timeout to Ok(None), other errors still raise
        if let Err(err) = &data {
            if *err.kind() == ErrorKind::Timeout {
                data = Ok(None);
            }
        }

        let enc_link = Self {
            link,
            peeked: data?,
            sealer,
            opener: Some(opener),
        };

        Ok((other_relation, enc_link))
    }

    async fn connect(
        self_relation: SelfRelation,
        other_relation: Relation,
        mut link: L,
    ) -> LinkResult<(Relation, Self)> {
        let timeout = Instant::now() + HANDSHAKE_TIMEOUT;
        let role = HandshakeRole::Initiator;
        let mut transcript = Sha256::new();

        // F1
        let offers: Vec<_> = offer(FLOOR_RANK)
            .map(|o| o.id())
            .take(u8::MAX as usize)
            .collect();
        if offers.is_empty() {
            Err(ErrorKind::Misc).msg(format!(
                "No available crypto suites, FLOOR_RANK = {FLOOR_RANK}"
            ))?
        }

        let mut selected_suite =
            crypto_suites::by_id(offers[0]).expect("Ids from offer should return extant id");
        let mut handshake = selected_suite.start(role);

        let mut f1 = Vec::new();
        f1.push(MSG_TAG_F1);
        f1.extend(HANDSHAKE_IDENTIFIER);
        f1.extend(random::<[u8; HANDSHAKE_NONCE_LEN]>());
        f1.push(offers.len() as u8);
        f1.extend(&offers);
        f1.extend(handshake.local_kex());
        transcript.update(&f1);

        // link.send(f1).await.wrap_msg("Failed to send F1")?;

        let mut msg = send_recv(
            &mut link,
            Some(&f1),
            None,
            &[MSG_TAG_F2, MSG_TAG_R],
            timeout,
        )
        .await?;

        // let mut msg = link.recv().await.wrap_msg("Failed to recv F2/R")?;
        if msg[0] == MSG_TAG_R {
            // recved R
            let new_suite_id = msg.get(1).wrap_msg("malformed R")?.clone();
            if !offers.contains(&new_suite_id) {
                Err(ErrorKind::Misc).msg("R not in offer set")?
            }
            transcript.update([new_suite_id]);

            selected_suite = by_id(new_suite_id).expect("id was in offer set");
            handshake = selected_suite.start(role);

            // F1'
            let mut f1p = Vec::new();
            f1p.push(MSG_TAG_F1P);
            f1p.extend(handshake.local_kex());
            transcript.update(&f1p);
            // link.send(f1p.to_vec()).await.wrap_msg("Failed to send F1'")?;

            // retry recv F2
            msg = send_recv(
                &mut link,
                Some(&f1p),
                Some(MSG_TAG_R),
                &[MSG_TAG_F2],
                timeout,
            )
            .await?;
            // msg = link.recv().await.wrap_msg("Failed to recv F2")?;
        }

        // Parse F2
        let f2 = msg;
        let (_nonce_r, remaining) = f2[1..]
            .split_at_checked(HANDSHAKE_NONCE_LEN)
            .ok_or(ErrorKind::Deserialization)
            .msg("F2 too short")?;
        let (kex_len, remaining) = remaining
            .split_at_checked(4)
            .ok_or(ErrorKind::Deserialization)
            .msg("F2 too short")?;
        let kex_len = u32::from_be_bytes(kex_len.try_into().expect("was split at position 4"));
        let (kex_r, sig_r) = remaining
            .split_at_checked(kex_len as usize)
            .ok_or(ErrorKind::Deserialization)
            .msg("F2 too short")?;

        transcript.update(&f2[..1 + HANDSHAKE_NONCE_LEN + 4 + kex_len as usize]);

        let mut their_hash = transcript.clone();
        their_hash.update(role.other().tag());
        if !other_relation.verify_digest(their_hash, sig_r) {
            // This also verifies that the peer holds the private key for the other relation
            Err(ErrorKind::Authentication).msg("Peer signature failure")?;
        }

        let mut our_hash = transcript.clone();
        our_hash.update(role.tag());
        let sig_i = self_relation.sign_digest(our_hash);

        // Send F3
        let (mut sealer, mut opener) = handshake.finish(kex_r)?;
        let serialized_relation = self_relation.relation.serialize();
        let mut f3 = Vec::new();
        f3.extend((serialized_relation.len() as u32).to_be_bytes());
        f3.extend(serialized_relation);
        f3.extend(sig_i);
        let mut sealed = vec![MSG_TAG_F3];
        sealed.extend(sealer.seal(&f3));

        let timeout = min(timeout, Instant::now() + (4 * SEND_INTERVAL));
        let data = send_recv(
            &mut link,
            Some(&sealed),
            Some(MSG_TAG_F2),
            &[MSG_TAG_DATA, MSG_TAG_F4],
            timeout,
        )
        .await?;
        if data[0] == MSG_TAG_F4 {
            // Verify ack
            let ack = opener.open(&data[1..])?;
            if ack != ACK_STR {
                Err(ErrorKind::Authentication)?;
            }
        }
        let peeked = if data[0] == MSG_TAG_DATA {
            // assume connection was successful
            Some(data)
        } else {
            None
        };

        Ok((
            other_relation,
            Self {
                link,
                peeked,
                sealer,
                opener: Some(opener),
            },
        ))
    }
}

impl<L: Link> Link for Encrypting<L> {
    fn scheme() -> &'static str {
        L::scheme()
    }

    async fn send(
        &mut self,
        msg: Vec<u8>,
    ) -> Result<(), impl std::error::Error + Send + Sync + 'static> {
        let mut ciphertext = vec![MSG_TAG_DATA];
        ciphertext.extend(self.sealer.as_mut().seal(&msg));
        self.link
            .send(ciphertext)
            .await
            .wrap_msg("Encrypting link failed to send")
    }

    async fn recv(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        let opener = self
            .opener
            .as_mut()
            .ok_or(LinkError::new().msg("Receive channel has been taken"))?;

        loop {
            let ciphertext = if self.peeked.is_some() {
                self.peeked.take().expect("Just checked if peeked was some")
            } else {
                self.link
                    .recv()
                    .await
                    .wrap_msg("Encrypting link reader failed to recv chunk")?
            };

            if ciphertext.len() == 0 || ciphertext[0] != MSG_TAG_DATA {
                // Ignore other msg types
                continue;
            }

            match opener.open(&ciphertext[1..]) {
                Ok(data) => break LinkResult::Ok(data),
                Err(e) => {
                    warn!("Decrypting error: {e}");
                }
            }
        }
    }

    fn take_reader(
        &mut self,
    ) -> Result<
        impl link_set::links::LinkReader + 'static,
        impl std::error::Error + Send + Sync + 'static,
    > {
        let opener = self
            .opener
            .take()
            .ok_or(LinkError::new().msg("Receive channel has been taken"))?;

        let link_reader = self
            .link
            .take_reader()
            .wrap_msg("Encrypting link failed to take_reader")?;
        let wlr = EncryptingLinkReader {
            link_reader,
            peeked: self.peeked.take(),
            opener,
        };
        LinkResult::Ok(wlr)
    }

    fn max_size(&self) -> u32 {
        self.link
            .max_size()
            .saturating_sub(self.sealer.overhead() + 1) // Subtract the length of the sealer's overhead and the msg tag
    }

    fn is_closed(&mut self) -> bool {
        self.link.is_closed()
    }
}

pub struct EncryptingLinkReader<LR: LinkReader> {
    link_reader: LR,
    peeked: Option<Vec<u8>>,
    opener: Box<dyn Opener>,
}

impl<LR: LinkReader> LinkReader for EncryptingLinkReader<LR> {
    async fn read(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        loop {
            let ciphertext = if self.peeked.is_some() {
                self.peeked.take().expect("Just checked if peeked was some")
            } else {
                self.link_reader
                    .read()
                    .await
                    .wrap_msg("Encrypting link reader failed to recv chunk")?
            };

            if ciphertext.len() == 0 || ciphertext[0] != MSG_TAG_DATA {
                continue;
            }

            match self.opener.open(&ciphertext[1..]) {
                Ok(data) => break LinkResult::Ok(data),
                Err(e) => {
                    warn!("Decrypting error: {e}");
                }
            }
        }
    }
}

async fn send_recv<L: Link>(
    link: &mut L,
    outbound: Option<&[u8]>,
    peer_prev: Option<u8>,
    expected: &[u8],
    timeout: Instant,
) -> LinkResult<Vec<u8>> {
    if let Some(msg) = outbound {
        link.send(msg.to_vec()).await.wrap_msg("Failed to send")?;
    }
    let mut last_send = Instant::now();
    let send_limit = timeout - SEND_INTERVAL;

    loop {
        let next_send = last_send + SEND_INTERVAL;
        select! {
            _ = sleep_until(timeout.into()) => {
                Err(ErrorKind::Timeout)?;
            }
            _ = sleep_until(next_send.into()), if Instant::now() < send_limit => {
                if let Some(msg) = outbound{
                    link.send(msg.to_vec()).await.wrap_msg("Failed to send")?;
                }
                last_send = Instant::now();
            }
            msg = link.recv() => {
                let msg = msg.wrap_msg("Link failed to recv")?;
                if msg.is_empty(){
                    continue;
                }
                let msg_type = msg[0];
                if expected.contains(&msg_type) {
                    return Ok(msg);
                }
                if peer_prev == Some(msg_type) {
                    if let Some(msg) = outbound{
                        link.send(msg.to_vec()).await.wrap_msg("Failed to send")?;
                    }
                    last_send = Instant::now();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use link_set::link_impls::{PipeLink, PipeLinkBuilder, PipeLinkHub};
    use tokio::{
        join,
        time::{sleep, timeout},
    };

    use super::*;

    async fn simple_link_pair(
        builder: PipeLinkBuilder,
        listen_rel: SelfRelation,
        connect_rel: SelfRelation,
    ) -> LinkResult<(Encrypting<PipeLink>, Encrypting<PipeLink>)> {
        let listen_addr = String::from("test_listener");
        let mut hub = PipeLinkHub::new(builder);
        let mut listener = hub.listen(listen_addr.to_owned());

        for _ in 0..60 {
            let listen_self_rel = listen_rel.clone();
            let connect_other_rel = listen_rel.relation.clone();
            let connect_self_rel = connect_rel.clone();
            let (connect_link, listen_link) = join!(
                async {
                    let link = hub.connect(&listen_addr).expect("connect failed");
                    Encrypting::connect(connect_self_rel, connect_other_rel, link).await
                },
                async {
                    let link = listener.recv().await.unwrap();
                    Encrypting::listen(listen_self_rel, link).await
                }
            );
            if let (Ok((_rel, conn_link)), Ok((_rel2, listen_link))) = (connect_link, listen_link) {
                return Ok((conn_link, listen_link));
            }
        }
        Err(ErrorKind::Timeout).msg("Failed to establish link pair")
    }

    #[tokio::test]
    async fn listen_connect_test() {
        let builder = PipeLinkBuilder::new();
        let listen_rel = SelfRelation::debug_get(0);
        let connect_rel = SelfRelation::debug_get(1);

        let (mut connect_link, mut listen_link) =
            simple_link_pair(builder, listen_rel, connect_rel)
                .await
                .unwrap();

        let msg = b"test_data".to_vec();

        connect_link.send(msg.clone()).await.unwrap();

        let recvd_msg = listen_link.recv().await.unwrap();

        assert_eq!(msg, recvd_msg);
    }

    #[tokio::test]
    async fn listen_connect_test_p_50() {
        let builder = PipeLinkBuilder::new().reliability(0.50);
        let listen_rel = SelfRelation::debug_get(0);
        let connect_rel = SelfRelation::debug_get(1);

        let (mut connect_link, mut listen_link) =
            simple_link_pair(builder, listen_rel, connect_rel)
                .await
                .unwrap();

        let msg = b"test_data".to_vec();

        let task_msg = msg.clone();
        tokio::spawn(async move {
            loop {
                let result = connect_link.send(task_msg.clone()).await;
                if result.is_err() {
                    break;
                }
                sleep(Duration::from_millis(50)).await;
            }
        });

        let recvd_msg = timeout(Duration::from_secs(15), async {
            listen_link.recv().await.unwrap()
        })
        .await
        .expect("message should arrive within this time period");

        assert_eq!(msg, recvd_msg);
    }
}

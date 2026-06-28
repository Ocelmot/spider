use std::{
    cmp::min,
    collections::VecDeque,
    time::{Duration, Instant},
};

use link_set::links::{Link, LinkReader};
use rand::random;
use sha2::{Digest, Sha256};
use tokio::{select, time::sleep_until};

use crate::{
    crypto_suites::HandshakeRole,
    error::{ErrorKind, Problem, ProblemWrap},
    link_impls::{authenticated::Establish, secure_link::SecureLink},
    LinkResult, Relation, SelfRelation,
};

const HANDSHAKE_NONCE_LEN: usize = 16;
const HANDSHAKE_IDENTIFIER: &'static [u8; 8] = &b"SPDRHS01";

const SEND_INTERVAL: Duration = Duration::from_millis(250);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

const MSG_TAG_DATA: u8 = 0;
const MSG_TAG_F1: u8 = 1;
const MSG_TAG_F2: u8 = 2;
const MSG_TAG_F3: u8 = 3;
const MSG_TAG_F4: u8 = 4;
const ACK_STR: &'static [u8; 8] = &b"SPDRACKD";

pub(super) struct Attested<L: SecureLink> {
    link: L,
    peeked: Option<Vec<u8>>,
}

impl<L: SecureLink> Establish for Attested<L> {
    type Inner = L;

    async fn listen(self_relation: SelfRelation, mut link: L) -> LinkResult<(Relation, Self)> {
        let timeout = Instant::now() + HANDSHAKE_TIMEOUT;
        let role = HandshakeRole::Responder;
        let mut transcript = Sha256::new();
        transcript.update(link.binding());

        // F1
        // Recv and parse
        let f1 = send_recv(&mut link, None, None, &[MSG_TAG_F1], timeout).await?;
        let (hs_id, nonce_i) = f1[1..]
            .split_at_checked(HANDSHAKE_IDENTIFIER.len())
            .ok_or(ErrorKind::Deserialization)?;
        transcript.update(&f1);

        // Check
        if hs_id != HANDSHAKE_IDENTIFIER {
            Err(ErrorKind::Authentication).msg("Invalid identifier")?;
        }
        if nonce_i.len() != HANDSHAKE_NONCE_LEN {
            Err(ErrorKind::Deserialization).msg("Nonce too short")?;
        }

        // F2
        let nonce_r: u128 = rand::random();

        let mut f2 = Vec::new();
        f2.push(MSG_TAG_F2);
        f2.extend(nonce_r.to_be_bytes());

        transcript.update(&f2);

        let mut our_hash = transcript.clone();
        our_hash.update(role.tag());
        let sig_r = self_relation.sign_digest(our_hash);
        f2.extend(sig_r);

        // F2 -> F3
        let f3 = send_recv(
            &mut link,
            Some(&f2),
            Some(MSG_TAG_F1),
            &[MSG_TAG_F3],
            timeout,
        )
        .await?;

        let (rel_len, remainder) = f3[1..].split_at_checked(4).wrap_msg("F3 too short")?;
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
        f4.extend_from_slice(ACK_STR);
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
        transcript.update(link.binding());

        // F1
        let mut f1 = Vec::new();
        f1.push(MSG_TAG_F1);
        f1.extend(HANDSHAKE_IDENTIFIER);
        f1.extend(random::<[u8; HANDSHAKE_NONCE_LEN]>());
        transcript.update(&f1);

        let f2 = send_recv(&mut link, Some(&f1), None, &[MSG_TAG_F2], timeout).await?;

        // Parse F2
        let (_nonce_r, sig_r) = f2[1..]
            .split_at_checked(HANDSHAKE_NONCE_LEN)
            .ok_or(ErrorKind::Deserialization)
            .msg("F2 too short")?;

        transcript.update(&f2[..1 + HANDSHAKE_NONCE_LEN]);

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
        let serialized_relation = self_relation.relation.serialize();
        let mut f3 = Vec::new();
        f3.push(MSG_TAG_F3);
        f3.extend((serialized_relation.len() as u32).to_be_bytes());
        f3.extend(serialized_relation);
        f3.extend(sig_i);

        let timeout = min(timeout, Instant::now() + (4 * SEND_INTERVAL));
        let data = send_recv(
            &mut link,
            Some(&f3),
            Some(MSG_TAG_F2),
            &[MSG_TAG_DATA, MSG_TAG_F4],
            timeout,
        )
        .await?;
        if data[0] == MSG_TAG_F4 {
            // Verify ack
            if data[1..] != *ACK_STR {
                Err(ErrorKind::Authentication)?;
            }
        }
        let peeked = if data[0] == MSG_TAG_DATA {
            // assume connection was successful
            Some(data)
        } else {
            None
        };

        Ok((other_relation, Self { link, peeked }))
    }
}

impl<L: SecureLink> Link for Attested<L> {
    async fn send(
        &mut self,
        msg: Vec<u8>,
    ) -> Result<(), impl std::error::Error + Send + Sync + 'static> {
        let mut frame = vec![MSG_TAG_DATA];
        frame.extend(&msg);
        self.link
            .send(frame)
            .await
            .wrap_msg("Attested link failed to send")
    }

    async fn recv(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        loop {
            let frame = if self.peeked.is_some() {
                self.peeked.take().expect("Just checked if peeked was some")
            } else {
                self.link
                    .recv()
                    .await
                    .wrap_msg("Attested link reader failed to recv chunk")?
            };

            if frame.len() == 0 || frame[0] != MSG_TAG_DATA {
                // Ignore other msg types
                continue;
            }

            return LinkResult::Ok(frame[1..].to_owned());
        }
    }

    fn take_reader(
        &mut self,
    ) -> Result<
        impl link_set::links::LinkReader + 'static,
        impl std::error::Error + Send + Sync + 'static,
    > {
        let link_reader = self
            .link
            .take_reader()
            .wrap_msg("Attested link failed to take_reader")?;
        let wlr = AttestedLinkReader {
            link_reader,
            peeked: self.peeked.take(),
        };
        LinkResult::Ok(wlr)
    }

    fn max_size(&self) -> u32 {
        self.link.max_size().saturating_sub(1) // Subtract the length of the msg tag
    }

    fn is_closed(&mut self) -> bool {
        self.link.is_closed()
    }
}

pub struct AttestedLinkReader<LR: LinkReader> {
    link_reader: LR,
    peeked: Option<Vec<u8>>,
}

impl<LR: LinkReader> LinkReader for AttestedLinkReader<LR> {
    async fn read(&mut self) -> Result<Vec<u8>, impl std::error::Error + Send + Sync + 'static> {
        loop {
            let frame = if self.peeked.is_some() {
                self.peeked.take().expect("Just checked if peeked was some")
            } else {
                self.link_reader
                    .read()
                    .await
                    .wrap_msg("Encrypting link reader failed to recv chunk")?
            };

            if frame.len() == 0 || frame[0] != MSG_TAG_DATA {
                continue;
            }

            return LinkResult::Ok(frame[1..].to_owned());
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

    /// Implementation only for testing
    impl SecureLink for PipeLink {
        fn binding(&self) -> &[u8] {
            b"Test Binding!"
        }
    }

    async fn simple_link_pair(
        builder: PipeLinkBuilder,
        listen_rel: SelfRelation,
        connect_rel: SelfRelation,
    ) -> LinkResult<(Attested<PipeLink>, Attested<PipeLink>)> {
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
                    Attested::connect(connect_self_rel, connect_other_rel, link).await
                },
                async {
                    let link = listener.recv().await.unwrap();
                    Attested::listen(listen_self_rel, link).await
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

use hkdf::Hkdf;
use rand::rngs::OsRng;
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::crypto_suites::ciphers::{
    chacha20poly1305::{ChaCha20Poly1305Opener, ChaCha20Poly1305Sealer},
    Opener, Sealer,
};
use crate::crypto_suites::{HandshakeRole, Suite};
use crate::LinkError;

pub(super) struct X25519ChaCha20 {}

impl Suite for X25519ChaCha20 {
    fn id(&self) -> u8 {
        0
    }

    fn rank(&self) -> u8 {
        0
    }

    fn start(&self, role: HandshakeRole) -> Box<dyn super::Handshake> {
        let ephemeral_secret = EphemeralSecret::random_from_rng(OsRng);
        let local_public = PublicKey::from(&ephemeral_secret);

        Box::new(X25519ChaCha20Handshake {
            ephemeral_secret,
            local_public,
            role,
        })
    }
}

pub(super) struct X25519ChaCha20Handshake {
    ephemeral_secret: EphemeralSecret,
    local_public: PublicKey,
    role: HandshakeRole,
}

impl super::Handshake for X25519ChaCha20Handshake {
    fn local_kex(&self) -> &[u8] {
        self.local_public.as_bytes()
    }

    fn finish(
        self: Box<Self>,
        peer_kex: &[u8],
    ) -> Result<(Box<dyn Sealer>, Box<dyn Opener>), crate::LinkError> {
        let peer_bytes: [u8; 32] = peer_kex
            .try_into()
            .map_err(|_| LinkError::new().msg("peer kex not 32 bytes"))?;

        let peer_public = PublicKey::from(peer_bytes);

        let shared = self.ephemeral_secret.diffie_hellman(&peer_public);

        let hk = Hkdf::<Sha256>::new(None, shared.as_bytes());
        let mut itor_key = [0u8; 32];
        let mut rtoi_key = [0u8; 32];
        hk.expand(b"spider itor", &mut itor_key).unwrap();
        hk.expand(b"spider rtoi", &mut rtoi_key).unwrap();

        let (send_key, recv_key) = self.role.order(itor_key, rtoi_key);

        let sealer = ChaCha20Poly1305Sealer::new(send_key);
        let opener = ChaCha20Poly1305Opener::new(recv_key);

        Ok((Box::new(sealer), Box::new(opener)))
    }
}

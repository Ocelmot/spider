use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce, aead::Aead};
use tracing::{debug, error};

use crate::{LinkError, crypto_suites::ciphers::{Opener, Sealer}};


const NONCE_SIZE: usize = 12;

pub(crate) struct ChaCha20Poly1305Sealer{
    cipher: ChaCha20Poly1305,
    nonce: u128
}

impl ChaCha20Poly1305Sealer{
    pub(crate) fn new(key: [u8; 32]) -> Self{
        Self { 
            cipher: ChaCha20Poly1305::new(&Key::from(key)),
            nonce: 0,
        }
    }
}

impl Sealer for ChaCha20Poly1305Sealer{
    fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        
        let nonce_vec = self.nonce.to_be_bytes();
        let nonce_bytes = <&Nonce>::from(&nonce_vec[4..]);

        let msg: Vec<u8> = self.cipher.encrypt(&nonce_bytes, plaintext).unwrap();
        debug!("chunk write with len {}", msg.len());
        self.nonce += 1;
        let mut ret = Vec::with_capacity(NONCE_SIZE + msg.len());
        ret.extend_from_slice(&nonce_bytes);
        ret.extend(msg);
        ret
    }

    fn overhead(&self) -> u32 {
        // 16 is encryption tag size
        NONCE_SIZE as u32 + 16
    }
}

pub(crate) struct ChaCha20Poly1305Opener {
    key: Key,
}

impl ChaCha20Poly1305Opener {
        pub(crate) fn new(key: [u8; 32]) -> Self{
        Self { key: Key::from(key) }
    }
}

impl Opener for ChaCha20Poly1305Opener {
    fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, LinkError> {
        let (nonce_bytes, ciphertext) = frame.split_at_checked(NONCE_SIZE).ok_or(LinkError::new().msg("Not enough bytes in frame"))?;

        // Decrypt buffer
        let nonce_bytes = TryInto::<[u8; 12]>::try_into(nonce_bytes).unwrap();
        let nonce_bytes = Nonce::from(nonce_bytes);

        let cipher = ChaCha20Poly1305::new(&self.key);
        let plaintext = cipher.decrypt(&nonce_bytes, ciphertext);
        if plaintext.is_err() {
            error!("Decrypting buffer returned {:?}", plaintext);
        }

        plaintext.map_err(|_| LinkError::new().msg("Error decrypting frame"))
    }
}

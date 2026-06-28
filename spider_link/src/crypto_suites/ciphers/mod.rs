

pub mod chacha20poly1305;

pub trait Sealer: Send + Sync {
    fn seal(&mut self, plaintext: &[u8]) -> Vec<u8>;
    fn overhead(&self) -> u32;
}
pub trait Opener: Send + Sync {
    fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, crate::LinkError>;
}
//! The SpiderId is a unique id that each member of the spider network must
//! have. The id is also the public key of that node which allows the
//! [Link] between nodes to be encrypted.
//!
//! The current bit length for the key is 2048.

use std::fmt;

use base64::{engine::general_purpose, Engine};

use rsa::{
    pkcs8::{spki, DecodePublicKey, EncodePublicKey},
    RsaPublicKey,
};
use serde::{
    de::{Error, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};

use num_bigint::BigUint;

/// The SpiderId that contains a generic number of bytes
/// to represent the public key.
#[derive(Debug, Clone, Eq, Hash)]
pub struct SpiderId<const BYTE_SIZE: usize> {
    bytes: [u8; BYTE_SIZE],
}

impl<const BYTE_SIZE: usize> SpiderId<BYTE_SIZE> {
    /// Make a SpiderId from an array of bytes
    pub fn from_bytes(bytes: [u8; BYTE_SIZE]) -> Self {
        Self { bytes }
    }
    /// Get the bytes from the SpiderId
    pub fn to_bytes(&self) -> &[u8; BYTE_SIZE] {
        &self.bytes
    }

    /// Interpret this SpiderId as a BigUint
    pub fn as_big_uint(&self) -> BigUint {
        BigUint::from_bytes_be(&self.bytes)
    }

    /// Interpret this SpiderId as an RsaPublicKey
    pub fn as_pub_key(&self) -> Result<RsaPublicKey, spki::Error> {
        RsaPublicKey::from_public_key_der(&self.bytes)
    }

    /// Encode this SpiderId as a base64 String
    pub fn to_base64(&self) -> String {
        general_purpose::URL_SAFE_NO_PAD.encode(self.bytes)
    }

    /// Make a SpiderId from a base64 String
    pub fn from_base64<S: Into<String>>(s: S) -> Option<Self> {
        let input = s.into();
        // general_purpose::URL_SAFE_NO_PAD.encode(self.bytes)
        match general_purpose::URL_SAFE_NO_PAD.decode(input) {
            Ok(bytes) => match bytes.try_into() {
                Ok(bytes) => Some(Self { bytes }),
                Err(_) => None,
            },
            Err(_) => None,
        }
    }

    /// Return the sha256 hash of the SpiderId
    pub fn sha256(&self) -> String {
        sha256::digest(&self.bytes)
    }
}

impl<const BYTE_SIZE: usize> Serialize for SpiderId<BYTE_SIZE> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.bytes)
    }
}

impl<'de, const BYTE_SIZE: usize> Deserialize<'de> for SpiderId<BYTE_SIZE> {
    fn deserialize<D>(deserializer: D) -> Result<SpiderId<BYTE_SIZE>, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(SpiderIdVisitor)
    }
}

struct SpiderIdVisitor<const BYTE_SIZE: usize>;

impl<'de, const BYTE_SIZE: usize> Visitor<'de> for SpiderIdVisitor<BYTE_SIZE> {
    type Value = SpiderId<BYTE_SIZE>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("SpiderId from sequence of bytes")
    }

    fn visit_bytes<E>(self, bytes: &[u8]) -> Result<Self::Value, E>
    where
        E: Error,
    {
        let length = bytes.len();
        match bytes.try_into(){
			Ok(arr) => Ok(SpiderId::from_bytes(arr)),
			Err(_) => Err(E::custom(format!("deserializing from incorrect number of bytes, expected {BYTE_SIZE}, found {length}"))),
		}
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut arr = [0u8; BYTE_SIZE];
        for i in 0..arr.len() {
            match seq.next_element()? {
                Some(val) => {
                    arr[i] = val;
                }
                None => {
                    Err(A::Error::custom(format!("deserializing from incorrect number of bytes, expected {BYTE_SIZE}, found {i}")))?;
                }
            }
        }

        Ok(SpiderId::from_bytes(arr))
    }
}

impl<const BYTE_SIZE: usize> PartialEq for SpiderId<BYTE_SIZE> {
    fn eq(&self, other: &Self) -> bool {
        self.as_big_uint() == other.as_big_uint()
    }
}

impl<const BYTE_SIZE: usize> PartialOrd for SpiderId<BYTE_SIZE> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_big_uint().partial_cmp(&other.as_big_uint())
    }
}

impl<const BYTE_SIZE: usize> Ord for SpiderId<BYTE_SIZE> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_big_uint().cmp(&other.as_big_uint())
    }
}

impl SpiderId<294> {
    const DER_PREFIX: [u8; 33] = [
        0x30, 0x82, 0x01, 0x22, 0x30, 0x0D, 0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01,
        0x01, 0x01, 0x05, 0x00, 0x03, 0x82, 0x01, 0x0F, 0x00, 0x30, 0x82, 0x01, 0x0A, 0x02, 0x82,
        0x01, 0x01, 0x00,
    ];
    const DER_SUFFIX: [u8; 5] = [0x02, 0x03, 0x01, 0x00, 0x01];

    /// Generate a SpiderId from a 2048 bit RsaPublicKey
    pub fn from_key(key: RsaPublicKey) -> Self {
        let pub_bytes = key.to_public_key_der().unwrap();
        SpiderId::from_bytes(pub_bytes.as_ref().try_into().unwrap())
    }

    /// Restores the SpiderId from its minimal byte form.
    /// 
    /// The modulus is re-prefixed and suffixed to create the key
    pub fn from_minimal_bytes(bytes: &[u8; 256]) -> Self {
        let mut built_bytes = [0u8; _];

        built_bytes[..33].copy_from_slice(&Self::DER_PREFIX);
        built_bytes[33..289].copy_from_slice(bytes);
        built_bytes[289..].copy_from_slice(&Self::DER_SUFFIX);

        Self { bytes: built_bytes }
    }

    /// Make a SpiderId from a minimal array of bytes.
    /// 
    /// Takes only the modulus of the key for serialization
    pub fn to_minimal_bytes(&self) -> &[u8; 256] {
        let bytes = self.bytes.get(33..33 + 256).expect("BYTE_SIZE == 294");
        bytes.try_into().unwrap()
    }
}


#[cfg(test)]
mod tests {

    use tracing::info;
    use tracing_test::traced_test;

    use crate::SelfRelation;

    use super::*;

    // Test the id's roundtrip
    #[test]
    #[traced_test]
    fn minimal_id_round_trip() {
        let id = SelfRelation::debug_get(5).relation.id;

        let serialized = id.to_minimal_bytes();
        info!("serialized: {:?}", serialized);
        let deserialized = SpiderId::from_minimal_bytes(serialized);

        assert_eq!(id, deserialized);
    }
}

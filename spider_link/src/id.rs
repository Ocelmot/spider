
use std::fmt;

use dht_chord::ChordId;
use rsa::{RsaPublicKey, pkcs8::{DecodePublicKey, spki}};
use serde::{Serialize, Serializer, Deserialize, Deserializer, de::{Visitor, Error}};

use num_bigint::BigUint;



#[derive(Debug, Clone, Eq)]
pub struct SpiderId<const BYTE_SIZE: usize>{
	bytes: [u8; BYTE_SIZE],
}

impl<const BYTE_SIZE: usize> SpiderId<BYTE_SIZE>{
	pub fn from_bytes(bytes: [u8; BYTE_SIZE])->Self{
		Self { 
			bytes
		}
	}

	pub fn as_big_uint(&self)-> BigUint{
		BigUint::from_bytes_be(&self.bytes)
	}

	pub fn as_pub_key(&self) -> Result<RsaPublicKey, spki::Error>{
        RsaPublicKey::from_public_key_der(&self.bytes)
	}
}


impl<const BYTE_SIZE: usize> Serialize for SpiderId<BYTE_SIZE>{
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

impl<'de, const BYTE_SIZE: usize> Visitor<'de> for SpiderIdVisitor<BYTE_SIZE>{
    type Value = SpiderId<BYTE_SIZE>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("SpiderId from sequence of bytes")
    }

	fn visit_bytes<E>(self, bytes: &[u8]) -> Result<Self::Value, E > where E: Error{
		let length = bytes.len();
		match bytes.try_into(){
			Ok(arr) => Ok(SpiderId::from_bytes(arr)),
			Err(e) => Err(E::custom(format!("deserializing from incorrect number of bytes, expected {BYTE_SIZE}, found {length}"))),
		}
	}

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>, {
        let mut arr = [0u8; BYTE_SIZE];
        for i in 0..arr.len() {
            match seq.next_element()? {
                Some(val) => {
                    arr[i] = val;
                },
                None => {
                    Err(A::Error::custom(format!("deserializing from incorrect number of bytes, expected {BYTE_SIZE}, found {i}")))?;
                },
            }
        }


        Ok(SpiderId::from_bytes(arr))
    }
}


impl<const BYTE_SIZE: usize> PartialEq for SpiderId<BYTE_SIZE>{
    fn eq(&self, other: &Self) -> bool {
        self.as_big_uint() == other.as_big_uint()
    }
}


impl<const BYTE_SIZE: usize> PartialOrd for SpiderId<BYTE_SIZE>{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.as_big_uint().partial_cmp(&other.as_big_uint())
    }
}

impl<const BYTE_SIZE: usize> Ord for SpiderId<BYTE_SIZE>{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_big_uint().cmp(&other.as_big_uint())
    }
}

impl<const BYTE_SIZE: usize> ChordId for SpiderId<BYTE_SIZE>{
    fn wrap_point() -> Self {
		Self::from_bytes([0xff; BYTE_SIZE])
    }

    fn next_index(prev_index: u32) -> u32 {
        let mut next_index = prev_index + 1;
		if next_index > BYTE_SIZE as u32 {
			next_index = 1;
		}
		next_index
    }

    fn calculate_finger(&self, index: u32) -> Self {
		let mut big_num = self.as_big_uint();
		let offset = 2u32.pow(index - 1);
		big_num += offset;
        big_num = big_num % Self::wrap_point().as_big_uint();
        let bytes: [u8; BYTE_SIZE] = big_num.to_bytes_be().try_into().unwrap();
        Self::from_bytes(bytes)
    }
}
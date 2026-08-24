//! Things related to the attach functionality
//! 

use std::fmt::{Display, Formatter};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce, aead::Aead};
use sha2::Digest;
use tracing::error;

use crate::{LinkError, LinkResult, error::{ErrorKind, Problem, ProblemWrap}};

/// Represents the status of the base when interacting through bluetooth
#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    /// Unknown status code
    Unknown(u8),
    /// Base is ready for operation
    Ready,
    /// Base is attaching
    Attaching,
    /// Base successfully attached
    Attached,
    /// Transient error, retry message
    Retry,
    /// Could not find network
    NotFound,
    /// Credentials/security parameters not valid
    InvalidCredentials,
    /// Connected, but could not obtain ip address
    NoAddress,
    /// Terminal system error
    SystemError,
}

impl Status{
    /// Converts the status to a single byte for serialization
    pub fn to_bytes(&self) -> Vec<u8> {
        vec![match self{
            Status::Unknown(code) => *code,
            Status::Ready => 1,
            Status::Attaching => 2,
            Status::Attached => 3,
            Status::Retry => 4,
            Status::NotFound => 5,
            Status::InvalidCredentials => 6,
            Status::NoAddress => 7,
            Status::SystemError => 8,
        }]
    }

    /// Creates a status from a byte as deserialization
    pub fn from_bytes(bytes: &[u8]) -> LinkResult<Self> {
        if bytes.len() != 1 {
            return Err(LinkError::new().problem(ErrorKind::Deserialization).msg("only accepts a single byte"));
        }
        match bytes[0] {
            0 => Ok(Status::Unknown(0)),
            1 => Ok(Status::Ready),
            2 => Ok(Status::Attaching),
            3 => Ok(Status::Attached),
            4 => Ok(Status::Retry),
            5 => Ok(Status::NotFound),
            6 => Ok(Status::InvalidCredentials),
            7 => Ok(Status::NoAddress),
            8 => Ok(Status::SystemError),
            _ => Ok(Status::Unknown(bytes[0]))
        }
    }
}

impl Display for Status{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result{
        match self {
            Status::Unknown(code) => write!(f, "Unknown: {code}"),
            Status::Ready => write!(f, "Ready"),
            Status::Attaching => write!(f, "Connecting"),
            Status::Attached => write!(f, "Connected"),
            Status::Retry => write!(f, "Retry"),
            Status::NotFound => write!(f, "No such network"),
            Status::InvalidCredentials => write!(f, "Invalid credentials"),
            Status::NoAddress => write!(f, "No IP address"),
            Status::SystemError => write!(f, "Internal system error"),
        }
    }
}

/// The type of the key used to encrypt the network details in transit
pub type AttachKey = [u8; 32];
/// The type of the nonce used in the challenge for the network details
pub type AttachNonce = [u8; 12];
/// A per-base salt for each session
pub type AttachSalt = [u8; 6];

/// An SSID
pub type SSID = Vec<u8>;

/// Version 0 of the attach details format
const ATTACH_DETAILS_VERSION_0: u8 = 0;

/// byte indicating open security type
const SEC_TYPE_OPEN: u8 = 0;
/// byte indicating WPA2 security type
const SEC_TYPE_WPA2: u8 = 1;
/// byte indicating WPA3 security type
const SEC_TYPE_WPA3: u8 = 2;

/// What kind of security the network should use
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachSecurityType {
    /// No password
    Open,
    /// Use WPA2
    Wpa2Personal(String),
    /// Use WPA3
    Wpa3Personal(String),
}

impl AttachSecurityType {
    fn variant(&self) -> u8 {
        match self {
            AttachSecurityType::Open => SEC_TYPE_OPEN,
            AttachSecurityType::Wpa2Personal(_) => SEC_TYPE_WPA2,
            AttachSecurityType::Wpa3Personal(_) => SEC_TYPE_WPA3,
        }
    }
    fn passphrase(&self) -> Option<&str> {
        match self {
            AttachSecurityType::Open => None,
            AttachSecurityType::Wpa2Personal(key) => Some(key),
            AttachSecurityType::Wpa3Personal(key) => Some(key),
        }
    }
}

/// The details of the network to which the base should attach
#[derive(Debug, PartialEq, Eq)]
pub struct AttachDetails {
    ssid: SSID,
    hidden: bool,
    sec_type: AttachSecurityType,
}

impl AttachDetails {
    /// Create a new, fresh NetworkDetails
    pub fn new(ssid: SSID, sec_type: AttachSecurityType, hidden: bool) -> LinkResult<Self> {
        if ssid.len() > 32 {
            return Err(LinkError::new().msg("SSID too long"));
        }
        Ok(Self { 
            ssid,
            hidden,
            sec_type,
        })
    }

    /// Returns the contained name
    pub fn ssid(&self) -> &SSID {
        &self.ssid
    }

    /// Is the network hidden?
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    /// The security type the netowork should use
    pub fn sec_type(&self) -> &AttachSecurityType {
        &self.sec_type
    }

    /// Decrypt the NetworkDetails recieved from the radio
    pub fn unpack(data: Vec<u8>, key: &AttachKey, nonce: &AttachNonce) -> LinkResult<Self>{
        let (nonce_bytes, ciphertext) = data.split_at_checked(nonce.len()).ok_or(LinkError::new().msg("Not enough bytes in message"))?;
        if nonce_bytes != nonce {
            return Err(LinkError::new().problem(ErrorKind::Deserialization));
        }

        // Decrypt buffer
        let nonce_bytes = TryInto::<[u8; 12]>::try_into(nonce_bytes).unwrap();
        let nonce_bytes = Nonce::from(nonce_bytes);

        let cipher = ChaCha20Poly1305::new(<&Key>::from(key));
        let plaintext = cipher.decrypt(&nonce_bytes, ciphertext);
        if plaintext.is_err() {
            error!("Decrypting buffer returned {:?}", plaintext);
        }

        let plaintext = plaintext.map_err(|_| LinkError::new().msg("Error decrypting frame"))?;
        let mut plain_ref = plaintext.as_slice();

        match plain_ref.split_off(..1).wrap_msg("Message too short, zero bytes")?[0] {
            ATTACH_DETAILS_VERSION_0 => {
                let ssid_len = plain_ref.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0];
                if ssid_len >32 {
                    return Err(LinkError::new().problem(ErrorKind::Deserialization));
                }
                let ssid = plain_ref.split_off(..ssid_len as usize).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?;
                let hidden = plain_ref.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0] != 0;

                // Sec Type
                let variant = plain_ref.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0];
                let passphrase = String::from_utf8(plain_ref.to_owned()).wrap_problem(ErrorKind::Deserialization).msg("passphrase is invalid utf8")?;
                let sec_type = match variant {
                    SEC_TYPE_OPEN => AttachSecurityType::Open,
                    SEC_TYPE_WPA2 => AttachSecurityType::Wpa2Personal(passphrase),
                    SEC_TYPE_WPA3 => AttachSecurityType::Wpa3Personal(passphrase),
                    _ => {return Err(LinkError::new().problem(ErrorKind::Deserialization).msg("Unknown security type"));}
                };
                
                Ok(Self {
                    ssid: ssid.try_into().wrap_problem(ErrorKind::Deserialization)?,
                    hidden,
                    sec_type
                })
            }
            _ => Err(LinkError::new().problem(ErrorKind::Deserialization).msg("Unknown version"))
        }
    }

    /// Encrypt the NetworkDetails for transit
    pub fn pack(&self, key: &AttachKey, nonce: &AttachNonce) -> Vec<u8> {
        let mut plaintext = Vec::new();
        plaintext.push(ATTACH_DETAILS_VERSION_0);
        plaintext.push(self.ssid.len() as u8);
        plaintext.extend_from_slice(&self.ssid);
        plaintext.push(self.hidden as u8);
        plaintext.push(self.sec_type.variant());
        if let Some(passphrase) = self.sec_type.passphrase(){
            plaintext.extend_from_slice(passphrase.as_bytes());
        }

        let cipher = ChaCha20Poly1305::new(<&Key>::from(key));

        let nonce_bytes = <&Nonce>::from(nonce.as_slice());

        let msg: Vec<u8> = cipher.encrypt(&nonce_bytes, plaintext.as_slice()).unwrap();

        let mut ret = Vec::with_capacity(nonce.len() + msg.len());
        ret.extend_from_slice(&nonce_bytes);
        ret.extend(msg);
        ret
    }

}

/// The security type transmitted from the base to the client
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkSecurityType{
    /// Could not decode this network type
    Unknown,
    /// No encryption
    Open,
    /// WEP encryption
    Wep,
    /// WPA encryption
    WpaPsk,
    /// WPA3 encryption
    Wpa3Sae,
    /// Enterprise encryption
    Enterprise,
}

impl NetworkSecurityType{
    /// Convert from a byte
    pub fn from_u8(byte: u8) -> LinkResult<Self> {
        match byte {
            0 => Ok(NetworkSecurityType::Unknown),
            1 => Ok(NetworkSecurityType::Open),
            2 => Ok(NetworkSecurityType::Wep),
            3 => Ok(NetworkSecurityType::WpaPsk),
            4 => Ok(NetworkSecurityType::Wpa3Sae),
            5 => Ok(NetworkSecurityType::Enterprise),
            _ => Err(LinkError::new().problem(ErrorKind::Deserialization))
        }
    }

    /// Convert to a byte
    pub fn to_u8(&self) -> u8 {
        match self {
            NetworkSecurityType::Unknown => 0,
            NetworkSecurityType::Open => 1,
            NetworkSecurityType::Wep => 2,
            NetworkSecurityType::WpaPsk => 3,
            NetworkSecurityType::Wpa3Sae => 4,
            NetworkSecurityType::Enterprise => 5,
        }
    }
}

/// The band the network is operating on
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkBand{
    /// 2.4 GHz
    Band2_4,
    /// 5GHz
    Band5,
    /// 6GHz
    Band6,
}

impl NetworkBand{
    /// Convert from a byte
    pub fn from_u8(byte: u8) -> LinkResult<Self> {
        match byte {
            0 => Ok(NetworkBand::Band2_4),
            1 => Ok(NetworkBand::Band5),
            2 => Ok(NetworkBand::Band6),
            _ => Err(LinkError::new().problem(ErrorKind::Deserialization))
        }
    }

    /// Convert to a byte
    pub fn to_u8(&self) -> u8 {
        match self{
            NetworkBand::Band2_4 => 0,
            NetworkBand::Band5 => 1,
            NetworkBand::Band6 => 2,
        }
    }
}

const NETWORK_DETAILS_V0: u8 = 0;

/// Details about a network that the base can see
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkDetails{
    /// The network ssid
    pub ssid: SSID, 
    /// the strength of the signal. If positive, a percentage. if negative, decibels
    pub strength: i8,
    /// The type of security this network uses
    pub sec_type: NetworkSecurityType,
    /// The band this network uses
    pub band: NetworkBand,
}

impl NetworkDetails {
    /// Create a new NetworkDetails
    pub fn new(ssid: SSID, strength: i8, sec_type: NetworkSecurityType, band: NetworkBand) -> LinkResult<Self> {
        if ssid.len() > 32 {
            return Err(LinkError::new().msg("SSID too long"));
        }
        Ok(Self { ssid, strength, sec_type, band })
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        ret.push(NETWORK_DETAILS_V0);
        ret.push(self.ssid.len() as u8);
        ret.extend_from_slice(&self.ssid);
        ret.push(self.strength.cast_unsigned());
        ret.push(self.sec_type.to_u8());
        ret.push(self.band.to_u8());
        ret
    }

    /// Deserialize from bytes
    pub fn from_bytes(mut bytes: &[u8]) -> LinkResult<Self>{
        let version  = bytes.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0];
        match version{
            NETWORK_DETAILS_V0 => {
                let ssid_len = bytes.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0];
                if ssid_len >32 {
                    return Err(LinkError::new().problem(ErrorKind::Deserialization));
                }
                let ssid = bytes.split_off(..ssid_len as usize).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?.try_into().unwrap();
                let strength = bytes.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0].cast_signed();
                let sec_type = NetworkSecurityType::from_u8(bytes.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0])?;
                let band = NetworkBand::from_u8(bytes.split_off(..1).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?[0])?;
                Ok(Self{ssid, strength, sec_type, band})
            }
            _ => Err(LinkError::new().problem(ErrorKind::Deserialization).msg("Unsupported version"))
        }
    }
}

/// Generate a random NONCE type
pub fn generate_nonce() -> AttachNonce {
    rand::random()
}

/// Generate a key based off a seed string. The key's entropy is limited by the
/// seed.
pub fn generate_key(hardware_code: &str, salt: &AttachSalt) -> LinkResult<AttachKey> {
    let stripped: String = hardware_code.chars().filter(|c|c.is_ascii_alphanumeric()).collect();
    let bytes = base32::decode(base32::Alphabet::Crockford, &stripped).ok_or(LinkError::new().problem(ErrorKind::Deserialization))?;

    // Checksum
    if bytes.len() <= 2 {
        return Err(LinkError::new().problem(ErrorKind::Deserialization));
    }
    let (mut a, mut b) = (0u8, 0u8);
    for byte in &bytes[..bytes.len()-2]{
        a = a.wrapping_add(*byte);
        b = b.wrapping_add(a);
    }
    if bytes[bytes.len()-2] != a || bytes[bytes.len()-1] != b{
        return Err(LinkError::new().problem(ErrorKind::Misc));
    }

    let mut digest = sha2::Sha256::new();
    digest.update(&salt);
    digest.update(bytes);
    digest.update(&salt);
    Ok(digest.finalize().into())
}



#[cfg(test)]
mod tests {
    use super::*;
    use rand::{RngCore, SeedableRng, rngs::StdRng};

    // ---------------------------------------------------------------- helpers

    const TEST_SALT: AttachSalt = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    const OTHER_SALT: AttachSalt = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];

    /// Payload of a notification at the default 23 byte ATT MTU.
    const ATT_DEFAULT_NOTIFY_PAYLOAD: usize = 20;

    fn key() -> AttachKey {
        [0x11; 32]
    }
    fn other_key() -> AttachKey {
        [0x22; 32]
    }
    fn nonce() -> AttachNonce {
        [0x33; 12]
    }
    fn other_nonce() -> AttachNonce {
        [0x44; 12]
    }

    fn ssid(s: &str) -> SSID {
        s.as_bytes().to_vec()
    }

    /// Build the on-wire envelope directly, so that unpack can be exercised
    /// independently of pack. Without this the two could agree with each other
    /// while both disagreeing with the documented format.
    fn seal(plaintext: &[u8], k: &AttachKey, n: &AttachNonce) -> Vec<u8> {
        let cipher = ChaCha20Poly1305::new(<&Key>::from(k));
        let ciphertext = cipher
            .encrypt(<&Nonce>::from(n.as_slice()), plaintext)
            .expect("test sealing failed");
        let mut out = Vec::with_capacity(n.len() + ciphertext.len());
        out.extend_from_slice(n);
        out.extend(ciphertext);
        out
    }

    /// Assemble an AttachDetails plaintext body, including deliberately
    /// invalid version and security bytes.
    fn attach_body(version: u8, ssid: &[u8], hidden: bool, sec: u8, passphrase: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(version);
        out.push(ssid.len() as u8);
        out.extend_from_slice(ssid);
        out.push(hidden as u8);
        out.push(sec);
        out.extend_from_slice(passphrase.as_bytes());
        out
    }

    /// Smallest body that can decode for a given SSID length: version,
    /// ssid_len, the SSID itself, hidden, security type. The passphrase is
    /// whatever remains, so it may legitimately be empty.
    fn attach_body_min(ssid_len: usize) -> usize {
        4 + ssid_len
    }

    /// Produce a hardware code carrying a valid checksum for `payload`.
    fn make_code(payload: &[u8]) -> String {
        let (mut a, mut b) = (0u8, 0u8);
        for byte in payload {
            a = a.wrapping_add(*byte);
            b = b.wrapping_add(a);
        }
        let mut bytes = payload.to_vec();
        bytes.push(a);
        bytes.push(b);
        base32::encode(base32::Alphabet::Crockford, &bytes)
    }

    // ----------------------------------------------------------------- Status

    #[test]
    fn status_roundtrips_as_bytes() {
        let all = [
            Status::Unknown(0),
            Status::Ready,
            Status::Attaching,
            Status::Attached,
            Status::Retry,
            Status::NotFound,
            Status::InvalidCredentials,
            Status::NoAddress,
            Status::SystemError,
        ];
        for status in all {
            let encoded = status.to_bytes();
            assert_eq!(encoded.len(), 1, "status must stay a single byte");
            assert_eq!(Status::from_bytes(&encoded).unwrap(), status);
        }
    }

    #[test]
    fn status_rejects_malformed_bytes() {
        assert!(Status::from_bytes(&[]).is_err(), "empty");
        assert!(Status::from_bytes(&[0, 0]).is_err(), "too long");
    }

    #[test]
    fn status_degrades_unassigned_discriminants() {
        // A base on a newer protocol can send a code this build has no variant
        // for. Decoding keeps the byte so the client carries on rather than
        // failing the read.
        for code in [9u8, 42, 255] {
            assert_eq!(
                Status::from_bytes(&[code]).unwrap(),
                Status::Unknown(code),
                "unassigned discriminant should degrade"
            );
        }
    }

    // ---------------------------------------------------------- AttachDetails

    #[test]
    fn attach_roundtrips_every_security_type() {
        let cases = [
            AttachSecurityType::Open,
            AttachSecurityType::Wpa2Personal("correct horse".into()),
            AttachSecurityType::Wpa3Personal("battery staple".into()),
        ];
        for sec in cases {
            let original = AttachDetails::new(ssid("test-network"), sec, false).unwrap();
            let framed = original.pack(&key(), &nonce());
            let decoded = AttachDetails::unpack(framed, &key(), &nonce())
                .expect("output of pack must be accepted by unpack");
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn attach_roundtrips_hidden_flag() {
        for hidden in [false, true] {
            let original = AttachDetails::new(
                ssid("hidden-net"),
                AttachSecurityType::Wpa2Personal("pw".into()),
                hidden,
            )
            .unwrap();
            let decoded =
                AttachDetails::unpack(original.pack(&key(), &nonce()), &key(), &nonce()).unwrap();
            assert_eq!(decoded, original);
            assert_eq!(decoded.hidden(), hidden);
        }
    }

    #[test]
    fn attach_roundtrips_ssid_lengths() {
        // Zero length is legal: it is the wildcard SSID.
        // 32 bytes is the maximum the standard allows.
        for len in [0usize, 1, 31, 32] {
            let raw = vec![b'x'; len];
            let original =
                AttachDetails::new(raw.clone(), AttachSecurityType::Open, false).unwrap();
            let decoded =
                AttachDetails::unpack(original.pack(&key(), &nonce()), &key(), &nonce()).unwrap();
            assert_eq!(decoded.ssid(), &raw, "ssid of length {len} did not survive");
        }
    }

    #[test]
    fn attach_roundtrips_ssid_with_trailing_nuls() {
        // The whole point of carrying a length: an SSID that genuinely ends in
        // NUL is now distinguishable from a shorter one.
        let raw = vec![b'a', b'b', 0x00, 0x00];
        let original = AttachDetails::new(raw.clone(), AttachSecurityType::Open, false).unwrap();
        let decoded =
            AttachDetails::unpack(original.pack(&key(), &nonce()), &key(), &nonce()).unwrap();
        assert_eq!(decoded.ssid(), &raw);

        let shorter = AttachDetails::new(vec![b'a', b'b'], AttachSecurityType::Open, false).unwrap();
        assert_ne!(decoded, shorter, "padding and content must not collide");
    }

    #[test]
    fn attach_roundtrips_non_utf8_ssid() {
        // SSIDs are arbitrary octets, not text.
        let raw = vec![0xFF, 0xFE, 0x80, 0x00, 0x7F];
        let original = AttachDetails::new(raw.clone(), AttachSecurityType::Open, false).unwrap();
        let decoded =
            AttachDetails::unpack(original.pack(&key(), &nonce()), &key(), &nonce()).unwrap();
        assert_eq!(decoded.ssid(), &raw);
    }

    #[test]
    fn attach_new_rejects_oversized_ssid() {
        assert!(AttachDetails::new(vec![b'x'; 33], AttachSecurityType::Open, false).is_err());
        assert!(AttachDetails::new(vec![b'x'; 255], AttachSecurityType::Open, false).is_err());
    }

    #[test]
    fn attach_pack_emits_the_declared_version() {
        let details = AttachDetails::new(ssid("v"), AttachSecurityType::Open, false).unwrap();
        let framed = details.pack(&key(), &nonce());
        let cipher = ChaCha20Poly1305::new(<&Key>::from(&key()));
        let plaintext = cipher
            .decrypt(<&Nonce>::from(nonce().as_slice()), &framed[12..])
            .expect("pack must produce a decryptable frame");
        assert_eq!(plaintext[0], ATTACH_DETAILS_VERSION_0);
    }

    #[test]
    fn attach_unpack_accepts_the_declared_version() {
        let body = attach_body(
            ATTACH_DETAILS_VERSION_0,
            &ssid("v"),
            false,
            SEC_TYPE_OPEN,
            "",
        );
        let framed = seal(&body, &key(), &nonce());
        assert!(
            AttachDetails::unpack(framed, &key(), &nonce()).is_ok(),
            "unpack must accept the version constant that pack writes"
        );
    }

    #[test]
    fn attach_unpack_rejects_unknown_version() {
        let body = attach_body(0xEE, &ssid("v"), false, SEC_TYPE_OPEN, "");
        let framed = seal(&body, &key(), &nonce());
        assert!(AttachDetails::unpack(framed, &key(), &nonce()).is_err());
    }

    #[test]
    fn attach_unpack_rejects_unknown_security_type() {
        let body = attach_body(
            ATTACH_DETAILS_VERSION_0,
            &ssid("v"),
            false,
            0x7F,
            "passphrase",
        );
        let framed = seal(&body, &key(), &nonce());
        assert!(AttachDetails::unpack(framed, &key(), &nonce()).is_err());
    }

    #[test]
    fn attach_unpack_rejects_ssid_len_over_32() {
        // The length is attacker controlled once the field is variable, so the
        // bound has to be enforced on the way in rather than by the type.
        for declared in [33u8, 64, 200, 255] {
            let mut body = vec![ATTACH_DETAILS_VERSION_0, declared];
            body.extend_from_slice(&vec![b'x'; declared as usize]);
            body.push(0);
            body.push(SEC_TYPE_OPEN);
            let framed = seal(&body, &key(), &nonce());
            assert!(
                AttachDetails::unpack(framed, &key(), &nonce()).is_err(),
                "a declared ssid length of {declared} must be rejected"
            );
        }
    }

    #[test]
    fn attach_unpack_rejects_ssid_len_past_end_of_buffer() {
        // Declares 32 bytes of SSID but supplies 4.
        let mut body = vec![ATTACH_DETAILS_VERSION_0, 32];
        body.extend_from_slice(b"abcd");
        let framed = seal(&body, &key(), &nonce());
        assert!(AttachDetails::unpack(framed, &key(), &nonce()).is_err());
    }

    #[test]
    fn attach_unpack_rejects_wrong_key() {
        let details = AttachDetails::new(
            ssid("net"),
            AttachSecurityType::Wpa2Personal("pw".into()),
            false,
        )
        .unwrap();
        let framed = details.pack(&key(), &nonce());
        assert!(AttachDetails::unpack(framed, &other_key(), &nonce()).is_err());
    }

    #[test]
    fn attach_unpack_rejects_wrong_nonce() {
        let details = AttachDetails::new(
            ssid("net"),
            AttachSecurityType::Wpa2Personal("pw".into()),
            false,
        )
        .unwrap();
        let framed = details.pack(&key(), &nonce());
        assert!(AttachDetails::unpack(framed, &key(), &other_nonce()).is_err());
    }

    #[test]
    fn attach_unpack_rejects_tampered_ciphertext() {
        let details = AttachDetails::new(
            ssid("net"),
            AttachSecurityType::Wpa2Personal("pw".into()),
            false,
        )
        .unwrap();
        let mut framed = details.pack(&key(), &nonce());
        let last = framed.len() - 1;
        framed[last] ^= 0x01;
        assert!(
            AttachDetails::unpack(framed, &key(), &nonce()).is_err(),
            "AEAD must reject a modified frame"
        );
    }

    #[test]
    fn attach_unpack_rejects_tampered_nonce_prefix() {
        let details = AttachDetails::new(ssid("net"), AttachSecurityType::Open, false).unwrap();
        let mut framed = details.pack(&key(), &nonce());
        framed[0] ^= 0x01;
        assert!(AttachDetails::unpack(framed, &key(), &nonce()).is_err());
    }

    #[test]
    fn attach_unpack_handles_truncated_frames() {
        // Reachable from an unauthenticated BLE write, so a panic here is a
        // remote denial of service against the base.
        let details = AttachDetails::new(
            ssid("net"),
            AttachSecurityType::Wpa2Personal("pw".into()),
            true,
        )
        .unwrap();
        let framed = details.pack(&key(), &nonce());
        for len in 0..framed.len() {
            assert!(
                AttachDetails::unpack(framed[..len].to_vec(), &key(), &nonce()).is_err(),
                "a frame truncated to {len} bytes must error rather than succeed"
            );
        }
    }

    #[test]
    fn attach_unpack_handles_truncated_body() {
        // Truncate inside the sealed plaintext to hit every field boundary.
        let name = ssid("net");
        let body = attach_body(
            ATTACH_DETAILS_VERSION_0,
            &name,
            true,
            SEC_TYPE_WPA2,
            "pw",
        );
        let minimum = attach_body_min(name.len());
        for len in 0..body.len() {
            let framed = seal(&body[..len], &key(), &nonce());
            // The requirement at every length is that this does not panic.
            let result = AttachDetails::unpack(framed, &key(), &nonce());
            if len < minimum {
                assert!(
                    result.is_err(),
                    "a body truncated to {len} bytes is short of the \
                     {minimum} byte minimum and must error"
                );
            }
        }
    }

    #[test]
    fn attach_unpack_accepts_empty_passphrase_for_wpa2() {
        // The passphrase has no length field, it is simply the rest of the
        // buffer, so a body cut to the minimum length is indistinguishable
        // from one that carries a deliberately empty passphrase.
        //
        // This is deliberate: unpack stays a parser and does not enforce
        // passphrase rules. The client rejects unusable passphrases before
        // serializing, and the base reports whatever the system returns when
        // it tries to apply one.
        let body = attach_body(
            ATTACH_DETAILS_VERSION_0,
            &ssid("net"),
            false,
            SEC_TYPE_WPA2,
            "",
        );
        let framed = seal(&body, &key(), &nonce());
        let decoded = AttachDetails::unpack(framed, &key(), &nonce()).unwrap();
        assert_eq!(
            *decoded.sec_type(),
            AttachSecurityType::Wpa2Personal(String::new())
        );
    }

    #[test]
    fn attach_unpack_discards_passphrase_bytes_for_open() {
        // An open network carries no passphrase, so trailing bytes after the
        // security type are accepted and dropped. Pinned so that a future
        // change to the layout has to acknowledge this.
        let body = attach_body(
            ATTACH_DETAILS_VERSION_0,
            &ssid("net"),
            false,
            SEC_TYPE_OPEN,
            "these bytes go nowhere",
        );
        let framed = seal(&body, &key(), &nonce());
        let decoded = AttachDetails::unpack(framed, &key(), &nonce()).unwrap();
        assert_eq!(*decoded.sec_type(), AttachSecurityType::Open);
    }

    #[test]
    fn attach_unpack_rejects_non_utf8_passphrase() {
        let mut body = attach_body(
            ATTACH_DETAILS_VERSION_0,
            &ssid("net"),
            false,
            SEC_TYPE_WPA2,
            "",
        );
        body.extend_from_slice(&[0xFF, 0xFE]);
        let framed = seal(&body, &key(), &nonce());
        assert!(AttachDetails::unpack(framed, &key(), &nonce()).is_err());
    }

    // --------------------------------------------------------- NetworkDetails

    #[test]
    fn network_details_roundtrips() {
        let network = NetworkDetails::new(
            ssid("visible-net"),
            42,
            NetworkSecurityType::WpaPsk,
            NetworkBand::Band5,
        )
        .unwrap();
        let decoded = NetworkDetails::from_bytes(&network.to_bytes())
            .expect("output of to_bytes must decode");
        assert_eq!(decoded, network);
    }

    #[test]
    fn network_details_roundtrips_ssid_lengths() {
        for len in [0usize, 1, 31, 32] {
            let raw = vec![b'x'; len];
            let network = NetworkDetails::new(
                raw.clone(),
                7,
                NetworkSecurityType::Open,
                NetworkBand::Band6,
            )
            .unwrap();
            let decoded = NetworkDetails::from_bytes(&network.to_bytes()).unwrap();
            assert_eq!(decoded, network, "ssid of length {len} did not survive");
        }
    }

    #[test]
    fn network_new_rejects_oversized_ssid() {
        assert!(
            NetworkDetails::new(
                vec![b'x'; 33],
                0,
                NetworkSecurityType::Open,
                NetworkBand::Band2_4
            )
            .is_err()
        );
    }

    #[test]
    fn network_details_encoding_is_five_bytes_plus_ssid() {
        for name in ["", "x", "a-typical-ssid"] {
            let network = NetworkDetails::new(
                ssid(name),
                0,
                NetworkSecurityType::Open,
                NetworkBand::Band2_4,
            )
            .unwrap();
            // version, ssid_len, ssid, strength, security type, band.
            assert_eq!(network.to_bytes().len(), 5 + name.len());
        }
    }

    #[test]
    fn network_details_fits_a_default_mtu_notification_for_common_ssids() {
        // Five bytes of framing leaves fifteen for the SSID before a
        // notification needs a negotiated MTU.
        let common = NetworkDetails::new(
            ssid("home-network-1"),
            100,
            NetworkSecurityType::WpaPsk,
            NetworkBand::Band5,
        )
        .unwrap();
        assert!(
            common.to_bytes().len() <= ATT_DEFAULT_NOTIFY_PAYLOAD,
            "a 14 character ssid should fit in a default MTU notification"
        );

        // A maximum length SSID still does not.
        let longest = NetworkDetails::new(
            vec![b'x'; 32],
            100,
            NetworkSecurityType::WpaPsk,
            NetworkBand::Band5,
        )
        .unwrap();
        assert!(
            longest.to_bytes().len() > ATT_DEFAULT_NOTIFY_PAYLOAD,
            "a 32 byte ssid needs MTU negotiation or chunking"
        );
    }

    #[test]
    fn network_details_handles_truncation() {
        let network = NetworkDetails::new(
            ssid("net"),
            7,
            NetworkSecurityType::Open,
            NetworkBand::Band6,
        )
        .unwrap();
        let bytes = network.to_bytes();
        for len in 0..bytes.len() {
            assert!(
                NetworkDetails::from_bytes(&bytes[..len]).is_err(),
                "input truncated to {len} bytes must error rather than panic or succeed"
            );
        }
    }

    #[test]
    fn network_details_rejects_unknown_version() {
        let network = NetworkDetails::new(
            ssid("net"),
            7,
            NetworkSecurityType::Open,
            NetworkBand::Band6,
        )
        .unwrap();
        let mut bytes = network.to_bytes();
        bytes[0] = 0xEE;
        assert!(NetworkDetails::from_bytes(&bytes).is_err());
    }

    #[test]
    fn network_details_rejects_ssid_len_over_32() {
        let mut bytes = vec![NETWORK_DETAILS_V0, 33];
        bytes.extend_from_slice(&vec![b'x'; 33]);
        bytes.extend_from_slice(&[7, 1, 0]);
        assert!(NetworkDetails::from_bytes(&bytes).is_err());
    }

    #[test]
    fn network_security_type_roundtrips() {
        for byte in 0..=5u8 {
            assert_eq!(NetworkSecurityType::from_u8(byte).unwrap().to_u8(), byte);
        }
        assert!(NetworkSecurityType::from_u8(6).is_err());
        assert!(NetworkSecurityType::from_u8(255).is_err());
    }

    #[test]
    fn network_band_roundtrips() {
        for byte in 0..=2u8 {
            assert_eq!(NetworkBand::from_u8(byte).unwrap().to_u8(), byte);
        }
        assert!(NetworkBand::from_u8(3).is_err());
        assert!(NetworkBand::from_u8(255).is_err());
    }

    // --------------------------------------------------------- key and nonce

    #[test]
    fn generate_key_is_deterministic() {
        let code = make_code(&[0xDE, 0xAD, 0xBE]);
        assert_eq!(
            generate_key(&code, &TEST_SALT).unwrap(),
            generate_key(&code, &TEST_SALT).unwrap()
        );
    }

    #[test]
    fn generate_key_ignores_case_and_separators() {
        let code = make_code(&[0x12, 0x34, 0x56]);
        let expected = generate_key(&code, &TEST_SALT).unwrap();

        assert_eq!(
            generate_key(&code.to_lowercase(), &TEST_SALT).unwrap(),
            expected,
            "codes should be case insensitive"
        );
        assert_eq!(
            generate_key(&format!("{}-{}", &code[..4], &code[4..]), &TEST_SALT).unwrap(),
            expected,
            "dashes are cosmetic"
        );
        assert_eq!(
            generate_key(&format!("{} {}", &code[..4], &code[4..]), &TEST_SALT).unwrap(),
            expected,
            "spaces are cosmetic"
        );
    }

    #[test]
    fn generate_key_accepts_crockford_ambiguous_glyphs() {
        // Crockford base32 reads O as 0 and I or L as 1, which is the whole
        // reason for choosing it for a code a human reads off a label.
        let code = make_code(&[0x00, 0x40, 0x00]);
        assert!(
            code.contains('0') && code.contains('1'),
            "test vector should contain both glyphs, got {code}"
        );
        let expected = generate_key(&code, &TEST_SALT).unwrap();
        let ambiguous = code.replace('0', "O").replace('1', "I");
        assert_eq!(
            generate_key(&ambiguous, &TEST_SALT).unwrap(),
            expected,
            "O should read as 0 and I should read as 1"
        );
    }

    #[test]
    fn generate_key_rejects_single_character_typo() {
        let code = make_code(&[0x9A, 0xBC, 0xDE]);
        let mut chars: Vec<char> = code.chars().collect();
        chars[0] = if chars[0] == 'Z' { 'Y' } else { 'Z' };
        let typo: String = chars.into_iter().collect();
        assert!(
            generate_key(&typo, &TEST_SALT).is_err(),
            "the checksum must catch a one character typo"
        );
    }

    #[test]
    fn generate_key_rejects_transposition() {
        let code = make_code(&[0x01, 0x02, 0x03]);
        let mut chars: Vec<char> = code.chars().collect();
        assert_ne!(
            chars[0], chars[1],
            "need two differing characters to transpose, got {code}"
        );
        chars.swap(0, 1);
        let swapped: String = chars.into_iter().collect();
        assert!(
            generate_key(&swapped, &TEST_SALT).is_err(),
            "the checksum must catch transposed characters"
        );
    }

    #[test]
    fn generate_key_varies_with_salt() {
        let code = make_code(&[0x11, 0x22, 0x33]);
        assert_ne!(
            generate_key(&code, &TEST_SALT).unwrap(),
            generate_key(&code, &OTHER_SALT).unwrap(),
            "the same code under two salts must not derive the same key"
        );
    }

    #[test]
    fn generate_key_rejects_unusable_codes() {
        assert!(generate_key("", &TEST_SALT).is_err(), "empty code");
        assert!(
            generate_key(
                &base32::encode(base32::Alphabet::Crockford, &[0x00, 0x00]),
                &TEST_SALT
            )
            .is_err(),
            "no payload left once the checksum is removed"
        );
        assert!(
            generate_key("UUUUUUUU", &TEST_SALT).is_err(),
            "U is not in the Crockford alphabet"
        );
    }

    #[test]
    fn generate_nonce_is_not_constant() {
        assert_ne!(
            generate_nonce(),
            generate_nonce(),
            "nonces must not repeat back to back"
        );
    }

    // ------------------------------------------------------------- robustness

    #[test]
    fn parsers_never_panic_on_arbitrary_input() {
        // Everything below is reachable from a BLE peer that has not
        // authenticated, so the only requirement is that none of it panics.
        let mut rng = StdRng::seed_from_u64(0x00C0FFEE);
        let mut buf = vec![0u8; 128];

        for _ in 0..20_000 {
            let len = (rng.next_u32() as usize) % buf.len();
            let slice = &mut buf[..len];
            rng.fill_bytes(slice);

            let _ = Status::from_bytes(slice);
            let _ = NetworkDetails::from_bytes(slice);

            // Raw garbage, rejected at the nonce comparison.
            let _ = AttachDetails::unpack(slice.to_vec(), &key(), &nonce());

            // Correct nonce prefix, so this reaches the AEAD.
            let mut prefixed = nonce().to_vec();
            prefixed.extend_from_slice(slice);
            let _ = AttachDetails::unpack(prefixed, &key(), &nonce());

            // Properly sealed, so this reaches the body parser with an
            // arbitrary plaintext. This is the path that matters most.
            let _ = AttachDetails::unpack(seal(slice, &key(), &nonce()), &key(), &nonce());
        }
    }
}

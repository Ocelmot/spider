use std::collections::VecDeque;
use std::io::Read;

use crate::{
    error::{ErrorKind, ProblemWrap},
    id::SpiderId,
    LinkError, LinkResult, SpiderId2048,
};
use base64::{engine::general_purpose, Engine};
use rsa::{
    RsaPrivateKey, pkcs1v15::{DecryptingKey, EncryptingKey, Signature, SigningKey, VerifyingKey}, pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey}, signature::{DigestSigner, DigestVerifier, SignatureEncoding, Signer, Verifier}, traits::{RandomizedDecryptor, RandomizedEncryptor}
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// The type returned by the Relation's sign function
pub type RelSig = [u8; 256];

/// The type of relationship of one member of the link.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Role {
    /// This member of the link is a peripheral, it can use any of the
    /// services the base provides.
    Peripheral,
    /// This member of the link is a base, if it is connected to a peripheral
    /// it can manage that peripheral. If it is connected to another base,
    /// it can pass data messages to that base for further processing.
    Peer,
}

impl Role {
    /// Serialize this Role
    pub fn serialize(&self) -> &[u8] {
        match self {
            Role::Peripheral => &[0],
            Role::Peer => &[1],
        }
    }

    /// Deserialize this Role
    pub fn deserialize(data: u8) -> LinkResult<Self> {
        match data {
            0 => Ok(Role::Peripheral),
            1 => Ok(Role::Peer),
            _ => Err(LinkError::new()
                .problem(ErrorKind::Deserialization)
                .msg("Invalid value for Role")),
        }
    }
}

/// The Relation includes both the id and role of a member of the network.
/// Typically represents the other side of the connection.
/// The local side of the connection is typically a [SelfRelation].
#[derive(Debug, Clone, PartialEq, Hash, Serialize, Deserialize)]
pub struct Relation {
    /// The role in the connection that this member fills.
    pub role: Role,
    /// The id of the network member
    pub id: SpiderId2048,
}

impl Relation {
    /// Creates a new relation from an id with [Role::Peer]
    pub fn peer_from_id(id: SpiderId2048) -> Self {
        Self {
            role: Role::Peer,
            id,
        }
    }

    /// Creates a new relation from an id with [Role::Peripheral]
    pub fn peripheral_from_id(id: SpiderId2048) -> Self {
        Self {
            role: Role::Peripheral,
            id,
        }
    }

    /// Returns true of this relation represents a peripheral
    pub fn is_peripheral(&self) -> bool {
        if let Role::Peripheral = self.role {
            true
        } else {
            false
        }
    }

    /// Returns true if this relation represents a peer
    pub fn is_peer(&self) -> bool {
        if let Role::Peer = self.role {
            true
        } else {
            false
        }
    }

    /// Returns a base 64 encoded representation of this relation
    pub fn to_base64(&self) -> String {
        let role: u8 = match self.role {
            Role::Peripheral => 1,
            Role::Peer => 0,
        };
        let mut bytes = self.id.clone().to_bytes().to_vec();
        bytes.push(role);
        general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Optionally returns a Relation from a decoded base64 string
    pub fn from_base64<T: AsRef<[u8]>>(s: T) -> Option<Self> {
        match general_purpose::URL_SAFE_NO_PAD.decode(s) {
            Ok(mut v) => {
                let role = match v.pop()? {
                    0 => Role::Peer,
                    1 => Role::Peripheral,
                    _ => return None,
                };
                let bytes = match v.try_into() {
                    Ok(bytes) => bytes,
                    Err(_) => return None,
                };
                let id = SpiderId2048::from_bytes(bytes);
                Some(Self { role, id })
            }
            Err(_) => None,
        }
    }

    /// Optionally returns a relation from an id from a base64 encoded
    /// string, and a role of peripheral.
    pub fn peripheral_from_base_64<S: Into<String>>(s: S) -> Option<Self> {
        match SpiderId2048::from_base64(s) {
            Some(id) => Some(Self {
                role: Role::Peripheral,
                id,
            }),
            None => None,
        }
    }

    /// Optionally returns a relation from an id from a base64 encoded
    /// string, and a role of peer.
    pub fn peer_from_base_64<S: Into<String>>(s: S) -> Option<Self> {
        match SpiderId2048::from_base64(s) {
            Some(id) => Some(Self {
                role: Role::Peer,
                id,
            }),
            None => None,
        }
    }

    /// Returns a string with the sha256 hash of the the relation
    pub fn sha256(&self) -> String {
        let bytes = self.serialize();
        sha256::digest(bytes.as_slice())
    }

    /// Generates a short signature for the Relation using the last 15 chars of
    /// the sha256 hash
    pub fn sig(&self) -> String {
        let hash = self.sha256();
        let char_count = hash.chars().count();
        hash.chars().skip(char_count.saturating_sub(15)).collect()
    }

    /// Use the public key in this relation to encrypt some data to send.
    pub fn encrypt(&self, data: &Vec<u8>) -> Vec<u8> {
        let key = self.id.as_pub_key().unwrap();
        let encrypting_key = EncryptingKey::new(key);

        let mut rng = rand::thread_rng();
        encrypting_key
            .encrypt_with_rng(&mut rng, &data)
            .expect("failed to encrypt")
    }

    /// Use the public key in this relation to verify some data against a signature.
    pub fn verify(&self, data: &[u8], sig: &[u8]) -> bool {
        let key = self.id.as_pub_key().unwrap();
        let verifying_key = VerifyingKey::<Sha256>::new(key);
        match Signature::try_from(sig) {
            Ok(sig) => verifying_key.verify(data, &sig).is_ok(),
            Err(_) => false,
        }
    }

    /// Use the public key in this relation to verify some data against a signature.
    pub fn verify_digest(&self, digest: Sha256, sig: &[u8]) -> bool {
        let key = self.id.as_pub_key().unwrap();
        let verifying_key = VerifyingKey::<Sha256>::new(key);
        match Signature::try_from(sig) {
            Ok(sig) => verifying_key.verify_digest(digest, &sig).is_ok(),
            Err(_) => false,
        }
    }

    /// Serialize this Relation
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(self.role.serialize());
        data.extend_from_slice(self.id.to_bytes());
        data
    }

    /// Deserialize this Relation
    pub fn deserialize(data: &mut VecDeque<u8>) -> LinkResult<Self> {
        // Deserialize role
        let role = data.pop_front().wrap_problem_msg(ErrorKind::Deserialization, "Failed to deserialize Role")?;
        let role = Role::deserialize(role)?;

        // Deserialize key
        let mut bytes = [0u8; 294];
        data.read_exact(&mut bytes).wrap_problem_msg(ErrorKind::Deserialization, "Failed to deserialize key")?;
        let id = SpiderId::from_bytes(bytes);
        Ok(Relation { role, id })
    }
}

/// A self relation functions similarly to a [Relation], but it also includes
/// the private key that corresponds to the id.
/// Typically represents the local side of a connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct SelfRelation {
    /// The private key of the local node
    pub priv_key_der: Vec<u8>,
    /// The Relation of the local node
    pub relation: Relation,
}

impl SelfRelation {
    /// Create a SelfRelation from a private key and a role
    pub fn from_key(key: RsaPrivateKey, role: Role) -> Self {
        let priv_bytes = key.to_pkcs8_der().unwrap().as_bytes().to_vec();
        let pub_bytes = key.to_public_key().to_public_key_der().unwrap();
        let id = SpiderId::from_bytes(pub_bytes.as_ref().try_into().unwrap());
        Self {
            priv_key_der: priv_bytes,
            relation: Relation { id, role },
        }
    }
    /// Create a SelfRelation from a der representation
    /// of a private key and a role
    pub fn from_der(bytes: &[u8], role: Role) -> Self {
        let key = RsaPrivateKey::from_pkcs8_der(bytes).unwrap();
        Self::from_key(key, role)
    }

    /// Generate a new SelfRelation with the given Role.
    pub fn generate_key(role: Role) -> Self {
        let mut rng = rand::thread_rng();
        let key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
        Self::from_key(key, role)
    }

    /// Get the private key of this SelfRelation
    pub fn private_key(&self) -> RsaPrivateKey {
        RsaPrivateKey::from_pkcs8_der(&self.priv_key_der).unwrap()
    }

    /// Use the private key in this relation to decrypt some data.
    pub fn decrypt(&self, data: &Vec<u8>) -> Option<Vec<u8>> {
        let key = self.private_key();
        let decrypting_key = DecryptingKey::new(key);

        let mut rng = rand::thread_rng();
        decrypting_key.decrypt_with_rng(&mut rng, data).ok()
    }

    /// Use the public key in this SelfRelation to encrypt some data.
    pub fn encrypt(&self, data: &Vec<u8>) -> Vec<u8> {
        self.relation.encrypt(data)
    }

    /// Sign some data using the private key within this SelfRelation.
    /// Returns the signature in byte form.
    pub fn sign(&self, data: &[u8]) -> RelSig {
        let key = self.private_key();
        let signing_key = SigningKey::<Sha256>::new(key);
        let sig = signing_key.sign(data).to_bytes();
        sig.as_ref().try_into().unwrap()
    }

    /// Sign a [Sha256] digest using the private key within this SelfRelation.
    /// Returns the signature in byte form.
    pub fn sign_digest(&self, digest: Sha256) -> RelSig {
        let key = self.private_key();
        let signing_key = SigningKey::<Sha256>::new(key);
        let sig = signing_key.sign_digest(digest).to_bytes();
        sig.as_ref().try_into().unwrap()
    }

    /// Verify a signature produced by this SelfRelation using the public
    /// key within the Relation against a signature.
    pub fn verify(&self, data: &[u8], sig: &[u8]) -> bool {
        self.relation.verify(data, sig)
    }
}

impl Eq for Relation {}
impl PartialOrd for Relation {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.role.partial_cmp(&other.role) {
            Some(core::cmp::Ordering::Equal) => {}
            ord => return ord,
        }
        self.id.partial_cmp(&other.id)
    }
}
impl Ord for Relation {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap() // There is no None option in partial cmp
    }
}

static KEYS: [&str; 10] = [
    "MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQDVaHIN5bllFFnuuG4-lmybsOceeM3KcwC2bErnLszSfeCmEjWnFFwlX1gKeS3__2IRAg_APgD6eN2z42U-f4l6c02BiETK_BAcrAhNftPXV3mv-RJEPgF3lD1bD7ZSG_Ik8knOBdpYb1njZzP8zU57xWhD_zcpUKe3eLqAU2u8Mh3yClO47a-GMw7mTDaKJGDRWGpL1aN-_s6qhZmd0XEjfH7E8nlqgyxWgrU3TWMP1rbYW8JWKEC_tivKddGicsRc01PRy4mE5_9sfjvJk2FphWbrtyxosQdMxU1zG9oGuHOcVexluSttae8gEEntz4LI6yeCd03rNB66hdlr1gt1AgMBAAECggEBAKflDBYy7bDAWiCdqN5Eqh2zB6HJmN31rFHY0PUgtLPFpMADA4L3WadtY26Z9763xQdsf8fXAB4OiR3FgRmybQ6ROCD4fGbV-DcWgVG2viNlBq-TXPOjdLQHRF4n9mCS6-Z1V-tmX2nD8QwfXZ8-RsjJfkZAu70dX1XQ_amH9_KOa2isoI1slHnXOfyW7iVg_Xjg2MraD9EID9ucYGltw8sJETDGnVK4HcCDMsFEcm3KUta5xWeWEMDohFaF5ywKVbdxVIJ7w-L6xuvFOmqoUdaazxH2wmqctFPsRchu03GSgFD6XfdNWFyQaNNsgZAkD5bQsqWI2Pqn0mw8i8SSTwECgYEA469GMtlRaes3LN7Dn661QobAp2iIL9FyRg1jkeawbK7_CjWXmbfENlxAulxU3CifuRHOHxvQ6KGBjaIBs2nHd6S1J2RTJPpSvHquGNmPpw2fpSUz9M7DvkJkwJNoS7ypbc3ja4HCvUcZPpimjtoAfzHH0Dp3RSGN9xOdUoB6R0kCgYEA7_KmNm6ABWKFvAmYL5oEsT-tj0W3X67cP6sX9fiZBwhs80KJPMHaPyPXMcRVfqt9sOkqtmgAti6GQe5pFk6qvCCCNdy6BPrCz80rgMxU7zuKsFqkUpCYzfuO74RH-Q584XwDcxle25IdBp_awEwk-JLOnAHBriCsU3vG2AYFRs0CgYEA2s_Mb_vIMTm7OeUQLbsSOdAVAA4Gq6Xm44nkgggozxpSwnYErtcbu35nOnKXn0lvTsXcyKrL13W3cu0aI1lqOAJTknrpKOVlc_uWqw0S8GG4ZlbdmszG82cNOsGvfvHeBkfS4rO--naEvVKo5yp9RcAKnoRBsW9w5t2z2ODeIvkCgYAXX5uxUbJG1AIS_xxEBszON1XAzxm9yFrMGO6Ml1rQxJFYYPLdaETKQcOEpYtue2YTBaOTgS4QkRei9IZoFeGr0b7dYFL-iS7Q7zt9oGnlo_culqXLJSq9ZfPWgRxBtpeTn3D2lVIYMXOCYa_9a21uiV528_TZ8XTX7QbBpFR_QQJ_QqDqXsseQpWvKk4cnLrt0qozfBfNzv6-1nS2KPXVpfLolax92yZvl2YWHKXDYixRgFpZ73N2YF_xSKUrKHqoyk3xFuyzMp2YAnZd5S3oUVyF2OqtnLhJTt6lthMsJNt0LcovqdX8a_Hgz_z1uRpohp7MMFFag7-AxjLlYA-ycQ",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCkbOGV0G9DLcNgpmSA7IlTQjZnbolYUzUAgy_udK3A00LEn3y1wxcs1dqMIYbDPbDh3VJFW5qdu3jSdyDzwwI6vymP7_pJRcsn5wvDINxmTrxMzk4FnaciPh-3Y56hUX1bpuEv9W-LqjmwswXdTLmOl7cjlh4FJeJ_Pi1ZAYzOk32U0P0OXG765JiRzi7m9CTNecYcMoulHo025kOgn4gYFvpqt746SNAAOmYQi5V8c-P-nxwGG2u-pOCMCCOcRND32ktLzAOnGZHxGQiFO9d553nak6hnoRI6H5e76NHxDqqgTI6920RM1fuluL_0tOTMxT_XJFBxCdmk_El-ql21AgMBAAECggEAIXlBG5zJaeXBJsX5I_4Dnv2V0czBDUgzPB14_pSmYuEfcKP_YYmMCEapLWoN42WFwxWpkBEsEvx9hmtWPAnxREByl6kFTTF2QHNbA6iG9hUFZKWDYMVNpzz6e_096B2M5cG9imPvB4HqKLzpbroV9J-SWx5OkkG6MKlZ1grovY0ZorrRS3LxH9mXl0HNU5xlod6Agls9oztzJNVD2cN2Wr-15aDZ1ekV_fyoTXbKxREMK83oEAV12k3e9mF-B2V6RxqRciF9xULaSKHveR3oFpPt-llv16CQP9OsG1qXM11zWIBDLbLunIGYX0XsdnqsRqseuCne0f9rG6RWAIJP6QKBgQDE9awDvgb0c0tTKPMk-Lo2a0C8zVElujn074hLoxYXKXuixN0VZHi3odcc7Ju4YVAUTb9_nYyge-wcjX9DA-itdLZJ5zBVlZOiyMb3pDAmjZj0IxxtoZ7-_mjFmbKeLtqqWgEZ4LNqE6vAjXMB2aQjD72nG7P1lexetevdYcGQJwKBgQDVtpU4b4c9dbPCuepW2G8_qC2OEcD4p5nBmnDzhOYdWN24BTOpW5voL_rckzQg4QsmmBTqnkDu9DYweclgwDPjuMXSj-CQqKHXc5u0pMU2HRTB2FN12cNzDFxfQeVpfAqBBhnOKdOworFBfJ4WSKvuV8T9QA1AawRcmOJq3mPwwwKBgQCVVA4O6EwtmhxJ-IogRdQo3jg-7QvRJtg6NEGJ1yQwe2sZhVh5l6tOzo1hiKKnsGAehLPj9XdhVZM2MrGCBbyrhgmPDpE-0iEVElSH_RvknwaQUu6C0D7T5d9ZsaYS-EMhVQvwqsRccH2Ph67igDhJvO11fTN7xydmx1cEidFPkQKBgQCC4baxQVxJv3O_paxmU1aOXajIgQb1QW9gqfzSpmlnP61JraXd6kSpBflUbLJYEHqLwEfPB-wsa1NkjLFPl2Yv6FD-iy60aRH0qNCK6P3-DgFQVfOHET4pj0Bi9jBRUa39JodXQzZpzrlPqcoHS6o_5XC2yCtVcDTToK3JVTlPkQKBgCR0BacWShAYEV_I9_yfg9ItX-hklm9y1VQeWpGEijxf3OWqwPIQc4iANJN0_UVXME6oCH6lL7UHujC89mhMBqfVqRGf4ENEVcyprnCXXB6MhBNOKi9aaRlVcXkJWmAWGMLdyUFgST3FVm0LRLNjKiCMw2NZpqFOBdGMwPuYaG_l",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDmzbbA_MHkvZunLFbnVu13dtKwqSNA1nznheOcgZDayGOxAYbQSbo7MxHmRipAylzb-0Pd1KQx5sXAp5WmXphv-gjnrJmUv_IpkddIOmKof-hnEqFpvH6mesaMh5XDAtp00QdsU7rU6R-MDwpUL1J0P9bmrEcOq6D-H72tje7awnOUj6b9s3IeJNGpF3QuSoLGTGkRytEWPoA6aiTK91aDuWrPyF9uGlssGkuNmyZyiDuD6aKW1WEbLPcVeXL7JFzTRC_qrOqbuEgca1muW36XcozIq_4Vl20WGqQSWtu_i-alt7gJb21J7D-5880TGWGTrjHDZ_YBh2FWM-h6IU_PAgMBAAECggEBAKIqQdDhBt7P9jCEb98Fbb31Z92mSVXCHmqR4TXSt1NxXtI8b1ujocz8egQgool5ZbtOlQWk5WUGb0WIuhX1-wcaV-1nkVU5dE2O4gMmurEHheP97BmdziLsutp8XVZyh8lyINQVFH2J-pdu7ePuh_GT9UuIGjkNkAVdiekKg5uBlMq6sH8t6a434ZaHp3kEEoRYpHS9Q3Ni-l_7O4EF-sT_8DfXc5jwpKbAAnplGiIVLj6U2i2rUg9Jq-fmVuEcB6D1DkWK_X8rOjgCFBD_L5J-QlBXNPeGy2J6bS6a5M0VDpPq7rCX2n3qy-SPSjhdRDdOIph5kkogRfm7t4sFzRkCgYEA_eGVHmqzsvEqyqKLdU6ycG__9X0dEBqpZ9K0s-yzOyXASGHwwkmKzq4j_wu5KmIi3k-V5CqUrGkROqAqejS4u3Yitlx2LVT80tZib9zoK6I-49bvygPOZ8C5XnOu8jgVc3m8BZ39p2-AVYEq5TftEd1HhlOmwGVAwML1b_KKMpMCgYEA6LrTdyJ6Lf-vgL_VcVAQ5OK0MB_yWha6LFeCrI8-4zz0601LAVFkOcC5EJnhStSeOacJWPKB2qAE5BVcUkrQ1QA3Sgd-YRwCPW6pigEzL6Jh3zO36lAFIB-XrfHAuZExUxI5F9V3o0B-N82c0d1SaciXBBXlHqmQy-Uh1QUBh1UCgYBbSJzhTuRF1sjYCxGxoBYwr1SM4-trOurmVbB7cQQQpEY6wFxcvyyVm699qK1vO9HltR_j5huG0lBLkAM15Xb2kEdy4lPrgL9W35aNOhSQe8m5CjM1o6C3VWhROa8RkHDGEGM2cdQeO80c8VCHElC_N5zcA3_VdZKOvIqbMc3W2QKBgAkRzs_jvhMw1awUzcKetinVU-RUOmOcYyD7QBJteqvsYjcRSg0BGQDPK-cjuA3sf0YL5mda_AiDF-2zj2d9lunWGlF-PUSXjNbMCztflJkUoO_L2iz9dVWtJYIX28TfjaaJHUR6gzEPgFu4XZYoI-APeyMjn0w0m3n2sfzVNxwJAoGBAJhf56zqBm8u-_7RncPqw6rfzz2EATNG1W_1ULdST6xBNhEN4HTLjiq7H8RA7K41wnT7X0awmJYrYOJkLk9VtbD368Uk9rqxQVAGHYIaDrNGKPLr8w4ECsNqQVyGTP_V_XDrJ5hgI62Epm2uZUZ2-nEGhGkYkXTDn-sB2EtiwRtP",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDewKNR-FnQ6sKk0SV2qpVM5aowV0zlOknBKxicmR2QtI0motF5LUZ3VAhMnqgfj8xe5pD_THUDqnkDQRdYqS7vO8Rynp6VvfNEPBziCDI0f4KH-XSnQhQVkNVkQON1SKoKKVnGbuGyTzteII15Ra6RlY5JEcPrACvJgjxZtAmVzNwgy5biROrFwRtfnPgREm9IhL10a9_SUJF_e7CNquAN1xLyk5cGB3PcaE1LG9SM5Zsz8x1kfC7Yih-QP2cjK2qkP-EehfwHI0Q-_gKVVni4Pgl-zEMyfSTIj_HoCNcn_7nXKx9sU1_lWcRUMNKCpJI_LC6TMa_HO9VY6zhB866FAgMBAAECggEBAIG9-mKYINhKpKyTFRsVKHjtnD8j2in51VOp4l_z3wCV6VEDrLbD5DNEwsC9-HbJruPnr7TDt7Q26t02YH7HrAqz6SxJr1zQkoy_5qLQ200wp7rDVWrGViRpg6EtGk8Jz-CzTRMDKnpNI-sjUsO8Dn3Femac7lxGcTqhnL0y3BJfdWwI3YA9qPxaQtVTr3REB7oYjmmCoH2KuN9CHBEVn4cStFjNdvYHPGAjA7VuBX4CxytFo9wwqcd7m63OAWZF27DmwYlMB3gC7lyoiPWACIOXAuRdvHp6tuHiKnDh0du8RNJejd7Z5_Mal-EfHq60rrwq-qxffO5wq2HORXU6pAECgYEA4DXBHrxMEilij3d1ojmj5lNnPm-LkbQXWFSBN-cp1QIVxqyOOqEHyLLjEkrNcStx0W5o5Xz96YPMW_s3AXXyNYZQZ6QqU-c4_W5Qijq3XKxf2WUqwEpVp-1qKO7vWmB1xSgwsBhxr7vIeAWpPzK5A4GTWae8W8GxYJMrqQFGj8ECgYEA_lX7CLuWAtkimJJLMkVJmICU12WMyFX__MT3VlGP0fSkmRs4rjeRUT7ufxv6njWyhrFqP2c8IbSHBJJrXQlt4WbHPv8-RvX4WCXABdjidLxHTVg2LNIpki281vuJ-iBtAcHCtz0UvvapZQPu7-cyJ1_h6dJO0wCzkW7LV1Lqz8UCgYAN84pWzUS2hJi7cKWDOK74MAxmmC5JHLmvJ2L7BYlW1dBhEm-vOkHvvSHgC4OJHTjx3TrtvL9X_nmC57jegGZX6kmqiU6Q9fxX2LtuPoUWYSt9rYvhdz6pOl62uVdvej0ZzYxqCLtaQgcRYNjNM-zLSQ7QL13LH9xXtBCtbYTkgQKBgQDGSxLlxBs5LUGj4qtuDkdK9zUUmsAgkax7zrVoPz76WtrZ5DdU3U4XIhGgWJgVMZh3G2vS8xIW1UFPdzjt2KQq0I8Xtrk_ahat4wDLjkVA7mpJCzVxzIlMxwwsMQFqWk2iyQafBqheGsIHWAG6WW7o9ACW5LlAGZPnF5LCCou4YQKBgFzW4k1aOAFuGcrI6gxRPVAAInSTT4QhPcG0Oud6nU_reTWjyR96Ug2mjG5s76WtmtiabFpne5Rkpp-kwEcHwiihlXJ2vrZ1vyw2U9qOqGDOa4Yssw3yDLFg-JAYF5fg7fcCu-VmyTeav5S-EPdImHut_qe_7h1Gz4_CnGr1t6e8",
    "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC3FXRzUl4SFYFaZnIH5sP0VYv697lp5sBPlBdUi7Xu06RwNb_8dULUWirPER0KbgH-tZucUEpsp2mceDJooHOVRIptvM5tnubryevSxChWs2QHd8GFx1rW8vCLXVfyCfqu4JOtOMaa3jYS86J76w4AWceEnk-zk-vCkkWIH-_mzrlzUIE1UxZ7KKvhk1uLuLppfFC2Gf8_JqRhH3mXzibgmSMKI1-EJkSFk5v8t2BWJ2BwpBJC8eP7ojkIwxcpq1bq85XaLNT9WJnNmfLAKx5Fr60z8JTmgiGrsuy51RlylLcedbm9Z2s_RXjxKqCZvn0OXcb80C4wDkHr-IFDAR7RAgMBAAECggEAOCxxCmEfU9UdVytPmXUIcpM828feL8wI_WGmtt4W-Cwfq_4R5dfkpVr0_4t0qZqPKiN2l0NbUnMbSFLoxIlWbVoWTw07GQ9EMtxFaumcpt1rt47a5b0A9iqb-2ascSr2q2lkZiWhair71FzEYdkA9sKVIO-h7KtaDtzqUJXb-ai3C-J4_Ph1IDYO8SnnP7OE99-SL507kqV1YSmuKQ9UooRov3P0NuES_K_A9_lWmwrM9pn1kkQ42BHfwcYNTjWwSo7HNKB85yzo8IAEBc4QShUPfYdaXixqjgv7GHVqP-KE-jyiPWOi9nQPGtKPk-eZEqmiUhA6OgKu4-gTjjFZQQKBgQDZLi7MsnZwSFiYw-k0-yXMl7C_ejHg9GCkpcGCg4E7J65sMyNG4nri83Y11F1PTZD8egUTPFZ5RUqVORAD-l5JETP9VuMn4cVXB9_Ldg3ua8JfNFYCqzS9nnzrz40p-xOkLYZTLSgrRiyIPfzzIBXXjzrDSzBThl2P5UPQhzo-BQKBgQDXzxFy7UbaIiuzFMr99QOEAU0lLnhMIEnx_-yQVCQ1y3C1Dylmj8dDp8YkuPMQNsIvmDhHPA42j7TiwlWJlQLTFHoPiDs-tOMpI57m5anQq6t9U2PhC0umbSwcDXazdeptRcF10LG5jirYk2FI-AOjEwoZflyHEQFma6b_lmLrXQKBgBKN5oOob4PyOld6zU6wci9Del8xcld0qVkHrDuZPo4uOrGVwNrKeJMxF5VLulkPGGbpict16TFdIR0UjfC5EBsP9DAdnzSGDlT8IDuCr3gCDs4Ra5O4yr20b5m51qaSg8AP_5zVi8v-p8lP-m9O-266FtwebVeFcDLd9Gg5VVl9AoGBALypdFqhXhhiWQukeNVM9fbX4GZJ2rbKX3qlPlzqggFZSb2vdIUJ2qylpk4CNdON23MaQtDbip1eQkcelwLA9wgq1Y5wjUKDhjc3wbmfOzaGbVQRq7ZYVpk2xaH8jzHSOs-udLMXb9eElqZhKWJOF3fftCuXUTJuxmeQYxz7jpytAoGAOzMhBCcHz85Hat8xOqB3rZn9KZjbX6CI9S8IOsJwnn02yAR8I2gjYZcztoY9XTkvY-8e_Z4AA5ZO05ZXCPAJjcqW1w6wxOFGPplW1ppQKnS4r3pwKWSyH-Q0lcXwuG73w5HxgW6NhOM7CjFG7Z-vCDcZJmBAXImxa_IZ87EgMEk",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQD2NdCundBAv_Z_fU_4F-lsvNSwVRRGTfmNRDKwvZblPr3EsDMHx1F5pwnb7l40ebo8j7sOmRDPsLhh36s-WEXdy9Gx014t54XCYLR-831UcUGtgsub4aK1-C4OLHCHtdLNrOq0VEE7dTC_gupA_FMCwzabaxDRXSFaMTzTrnUIj7egyxBxfKJL8Xo3YnetmxA7TyNhBMcHnpwHghczVIrn13n7b_WYtOBTKd6C4cuLcIs7KolP9UWJIP_pAdAihGkQ8sLOPf2Eug7o9b9mUJBP21NJNVgZKowLplPBaOJueQkxWSfrSYDDBBULsltGwToB7AYz_HLWhD6_S2VVTcrjAgMBAAECggEAAjtGou6HPq7-uvUVznfZA8VKYDbFMkXupxshInz1ayqAadH3BHEICgQ7kKS-cddQms_qsB0vC9LcOPbgBHakW3hHdEcoKV5Z1gMuX6AxWJyWLc5rrgQDh4ayVCqO8ovxVy_kCLJizPko8fNr0B75WaMPUaVMx7kCXmDn3jtMkEFIqbm_YJiZP7bd2jBMUhS0hh-K9lwJI2MpqCfZHm2RK2PLSV_HGNRjYMrZRCFVAe1xU-EfJX-V-b6kqc3vsuHe4d1v5-thLsYhTEMuuKtJ6wbiSV3u-93NfxQGiQ_UPfs8poDc68PeOgQt1G9LsmwK0tXlrJZRbbp_hweNbcy70QKBgQD6mZ-cj21oQjkaVTZ0pYSjs3qVKEf3viSi7ZORZuT-ym_t23u-g5AoEG4l2KblAO7f7CAd0WBzMGwJazt_0uQOYaqM8BG1svNXaF0I8zuLOYZGxhFFVbeR8La5mXc08cYEf_dUFyuZuwtreyM3e-dAgtQrNPjyPiTZ5szIawBEbwKBgQD7g_nYlVslN1qbw_BRTlNcqI5E_2skAc3-kRg1hJkYM2Zk1V4IRn8iok_zt8N18b8wV3dyH3NupgQWMM1sZxWuaHbaxzVJeic5fPER4WhgUXYZVVzPngvRNrl3NZQGWZVwjIcjuDG7HxG5rWFmSM8YJgBc8TnMbU55vHJZGaPizQKBgQDj7sDvbMFNeBZqLiFmvXnET6XqbwXuf2LhUofLU7RrTwO5a83EvfNrjW4yPDmox01-HE4l8N_yRZOuiXtHyzClKA4xPNZO1uJgmUstrdZ1zq-kRdlFoC5krnX0oHJ9lH7Qbvgt4xlELY24h_rDJ45x7c0_M2JPK1jnXPbcP_6xPwKBgEubl97Qvy6H3lgW5cY7Snn_PY1mTtnrJPaSvXlRHAiXYv-K2JKaRputuWUlZ7-r5XJtyL1o0PWBOJdHImmk73KXeqs32T_2VZZFhd5_KTZTJrJk49qRwzGoRsacN5xVD8Reqj1FoMWEiHqNsUrjNkYnHlLSPTLD5SRZTvKT7509AoGBAJ2JgbE_AOjQYqeAX2vlSzgOKwvQQO0K8hVpHOeTwj5mEDwdYAWMYMsQK-Pl1PAwfRNM4mCgLHiQIoJmnlpr1skp8PyvS25UlC7gL4NzN947F6D2VL5K1fTuKBOPGroLmIs2bPFKO0CXNPZHN7ngf2D7dmYY_JkLhfx-EGjAGs7a",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDtlWtY0ztGEQgySut8HECP_dITvX2sRGifsTblThfhghNOOuD3frcq3XLWCenuAelcCqUPkUkMJh4DmArVKMbnQ5ZvSHqMdR752L7ca6zvjBtGSeuQXL1nljXhMjfvVDUfuzT8gSyqtSEAWcQoKqXpc_EkVrgogWQKBJhqYXKSugKOGUIeuwEaCaLg0QsjTfy4JpCHz6A9tM0NDpg3Z4hd_xA_Qqira7-48FJEO6auwijbCRIW3eyEMbbwDbZVT8u_LyYPS_J-sDRarW9G2Ftr_VAqztRjGjHYJNxtpXr0h1T332Vp9AsB9olARFnOEbf_vOd-yeLwApP0oFoePT8ZAgMBAAECggEAFVlE4SFyBRTIuMQ0rt1XC9lSBEYVweIPyLHC7g37ZV8r3u65gmPXj7mAdS1E4Qc48fVe6awzdS0Dq20BJDBa1zMilHNd17s6glbwp5vhWVEsrj88NKewuVstEkRR9GaLs4M9-qac3eYSxhZK4xUZ_YVWmN5WBAXEeIX7MCv7gKWb992uPr2cHW-Rce9PVfd5UZjvnp80pqI17qClrRGUcUnTIGj52B6it81Pq71Nyq6JFNF3msrX3TaUwBjV8XysNJNISulbzIiFF7xvquGeLYMw4ufiC__8uXrtBOycJKJv43gQnqCK_ZlF6eP4HTY2zhIFTTK7PL062EgBaJSXtQKBgQDvgpL49CWXk5UN-y-0bVdyMkbCGi8TupOGwcDMKjZoMA8JhC-xnoUlSDsyYHRfOrmLLZbhxObZ72RB28YHt6IB8S628iAiy3ncOBkkNqmohOka7JazbXSN5O2DRX0uXCfI-kODixHIuVNlWKWsnl-0PFvcR_Kc28zwF2jv-P9PLwKBgQD98ORlOu9cFzopdWYpv_g5XBZwmATTB2r6AoIqkc93x2vxl_0WkAgVOfn2PhGMOY0U_V16bT7QgMlfZrgCU6ombiuxfEzy32iGo0YHMP2S_UbKbk7dtxdJCDpTd_xvJNiMCchkg5aBe_cES1jcsS5MFhyavtwfoVbTkqugMqGENwKBgQCtvRZIKTKrxY62WaO9SiPI3tedLclAknM5qYrljylwYoxF5vGB8u-6n67xWC6SddLqNuPgWijrplAfxgDc0ERhDEdKxlCxbNC1AqyaLFzdtawyr7SR67BEze_M7bdkzcy-aWxYG6WG6YipV6i2kxvxbmdkX3yKdJcxAopIqWLqWQKBgQCWMKhUuvOgithKdvXykWiVPELFWxPXYBbEQUGNPenv1NGh9RuqAYvWShDts64bOlqX5HYqF3zEQrdXJmCEd8k1q4lKEtNL_hhLMTwUusPu8L-ysGUSutwZxLUCcv-pGKi-wnZ0BGO6t3_UWV_4Pw67z4QhfeqhBoJc5e199RQUJwKBgAF8d90VjmAyPA1P3nDXIAokwyWj-0cMg5X_lIb35xH1X-oGuoPQgYpl9Ozj51i0v3NN3ti74AwP98XZf5ejNoqQqLfAZl6VYsaRadu42mJhNUTx4Hdi7pi6haB1FGQP7RTJSQI023pP6LR_TNPXoj-h2EQDtP1aPtZ-sdS6noFK",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCzjahE0klCWfDQPOXwhKY3441Bhddb4POeeWSgGPCyvMP4GAjat7etx5IM2yfZ3crFMchLDqqz3MDU1mKaBKnJGvm8m1Ap7UM-sDNVJXH8o9a3dINslDaktGvUEqTnnnoxrRcmWhOiUCgua2jfAn4blX_a9LLbSs5AWapaD7TS2JgU1KiB5BcX-66kB4brrc_7u704FgRgXuLPAjdXJoygAArkSRIUzlZiA4LJwIK4AJuTyqTDdzvfNULWIa4GPGFE8RncCSmV8KM__mRNUzA1PQkelNW7T5Di9qVzsUgkf_y6jWZRfNy5YPdVDXsa8ihodRdEW-ofCCp-xER667uNAgMBAAECggEAZHoHCcwrVsgB7GXHvtpcMbZS27HMHAw8CBiiaLzMJRlhCLpaahqY3NRrNGqHWHG3ALalAOuKNvB3sCLwNoQZhwJjclnveCbflHsBnso_iUMd6rd2vBIMqgqUtK5iPYL_mkDkTX_msNPuSFuu6ez5KPJ2A88fL3wjAvuqSo-zfrDpZ3kvh8DCbqbSX6Nd6Hxy_r5tHL9K2II8DLXQm9vMucTydINDOgyiGNIyep0GfnPBvrT_YGE2Vi97CPyYzk8l2KF2gSmnHllsyiMHPUKJcNr-8x5WPBAgigOENQjKlk8SgqcHyHa98cXvDI79ufeOhEtEekLJ4GZklQ_hxX1o-QKBgQDGKDAUrkfCyT3LBHVPgjAkJ3tUuN9A3jK4JCDBB7-y0KtfylUAG4VpoWBt_UbBR8cvq8P_tosIv3Lc1sl7Yk_zGoWkNNhBmP1mt9CXwr8f58PPqlygAz4O_NPJ9LmsIt-rRimkNXhhJ5SYckUhqWCrPeXRgHmO94PBR5SiDaSsNwKBgQDn90NRtygMVF-I3FSf1xhRsIejLhzQNtiikhK_qafgWLjAiPjVKkZB3JIU8ldDvRO2Pp6xEzqVuEFEHHgrao8Lj9J7kXBex-i63d8wmMAMpi18-1Zs9ZPMvfhtya2-xm9EnNLQT6eunuiNRjm1ntIGyxU8Qtln_XOKGN-z7JecWwKBgAMhIVlCQ1ndKb-qC1w9FxuLEBSPct8oYy6rS2UhBTMCvqIdpOgCh0HazZYPGO32fzvOf6LrNBjoRR0du0Lak56oPZIRT5UBoIbdwkqTRcdwNpRyAVJ5mqJL9eBLoB8K0fN3gTLZsVP_dwZNT7AvZ5psuNNtLv5GBGuqALvqcT5TAoGBAOYMJ7xLu7D2oERX1qkqpZimTO49Vh_8tL4NxgBEnhP1iUyQys6E_WZl_I4_hOHOC1WLJtCQNGK1eCy3W0obhL-_o8wegeXNtnZUgfttEdG5oJU5og2vQyQjJtSi1efEXicarEwhIzgfqpwpECSZ3MsV7vpzha8HAeXsIzcKJZXPAoGBAKivDJrx2Kp0uh7DaDdnDkkSPAd91DnBhKMjOSjpNws8g9tqvOevg20NjMe-9QFdU6jVPpHpgN1eklSrDzoy0Kcy-scc27ANZiGFTDmwCKPpUFNdB4j5uXOl8XU0ofG9PIQNOLAEQ_Z6nEkDhqVuZxqRNNXA_3mgFjKvPg3i1B7o",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDCSRNgE1NhUQCGZMriA92zpnk9lA4FRRZCEtzREEfXBMnGDxxWOpBKlX1ScupZ6Lm3eShG6VX7fMy1SHgrijSa1PJ5R7Dh_YpfXBL6UkfrQQDjwgXadRF-1Yixh-yRK9LdFCJlBGSEit50Wjb-8D-JC9MOOjM28RT6k2WonHGGhYoN76AU79jNO8EktAt39tx8TNHoou9nfoA9w80C2SW5XpF5P6IoTPI5dIF5KvNNUJRRwl0flkm7aOJJKQL4Mo1szX0dIK6UoniGodkj3_TvLQQQwPV1Hl-xgQm1GTmCue_gsiuVCixYG1EPuqq3GLsARNiIDiCRhPBc02aX76r_AgMBAAECggEBAKhVhnsU44aLF2haRkrjzKCOWbfX4vn_7RQcocRmVchgNq7rgsLXhROKSIY6WQDDmr30NMiT_VKjw_5CJxSab4_Dd79LNV_pPI35CdxnlDaqIKo1_rpT-m-pdgFT4s25ab7xZgeecbVBWRL443OqF3KXkytdk-hDo8ikE02vFtHUuVT_5bWqe6chltKpvCifBvjjucelJHLJkYSzt-HTAjIK4wOBiQkUsCUs4sFeHHCjkmDQav6GoAc_N0-iCpmI0g17FrNM7MUNiSf_xa001T-iDNLPsu0VLDtILtTSAo6Fe2UttF5UFk5hPk1gTia9ardggU5LrxwjhjAD0v--gdkCgYEA6wjgC0gi5kYidaUWJ6GKu2h5qwC7nUo6426qn6iniOsEOVJfMLvW_3CBKBqzvP7FrlhROryGsQaYN_wzpZMaw_IOExt1QTnZKqmJGoWPNo2O53bRTsKtDCJ7Mpo9gYN-8ejdneWeOk2wYvsNgPRVA5eevv9_ZJroK99-k3apuVUCgYEA052sY5DaBAkaCNTvgG9ZxwJY3iuaGcFWgqVDj_rebZgNsBqzm_LUgHmt7LhaxilktAK5wshCDCPGR6c3AYZZdCAQto2A6_bB3UcW2kAdYcylPxdUhxEwyrfKUxApG6uuRaKQCE8PzyJ2hcNHeC8fA4nVh4oBOj6CZFh_a4GsgwMCgYAnPqkiWR4ysx1H5ZPodCnVFyHRsuKg5eclWLI2zJOE4jEnXSC143eH2YJHbwX6FdRuQyL1GsumvYInPv5ktEZw13cQK7KNfJpNbFePTSPXqRVmgsl6TDlW4F8P2P9SI-HLhOWUWuXruFMug9sCYEndurBwFftwkgkYYk4hU1wNrQKBgF_Xf6Ywiq1dOe9aEYcH549cnscw1EBp8jaFhw4Evwy_2yMxVLuCxX_SnFUkQeiSAswMl_mCHXfGFB2LvDvyjz0q8Kbde9BjF_aSJeV_OE99EjJ67Iun24NUrkvke9nObcWYFMfOlwhnQWrfwNTL8q31lUIn3Np2STQNaNhWCL3lAoGBAJqciVW1zAghVNLi3Fuq3PVxCpjVK9YC69Gyl9P1mpVJw2xWjujOLIci0wr0rO5gQ0oFK3X8itR8nDmt_Na69UO11w5bZybEuwLrkaPp2qPTuZn9coifveKleZwrYFAfnAM31FxpnvaCm4D1cwA7Fc1HNjOcGfsUbELQ3Fcq96lS",
    "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCaTW4hXvP3rWe8Wiqa0rzkYgRhl-HGg0RotRLBDjxHhUWjHRff7KASA7tDEWkrJitQQDn6YF4PStepYGVEfi5InnKnIhvCPasoqnAOXHKx-wbkijRHvf61gqpQBMwQUZBy4se-BqZ8jcEDa_yzOxgMkWWQZLNjz8kEkeBWnGuc5yXOcl_RGHKOZWu3wtHHKeo2KntEORIsfhyN64Cyne5IEYXd3dvGAgZElOo_-tGBZEgwt_XlpugVQepwEud1-hzISN1HSTT2WOQKQ3ogDBvlRVwMPrAxZH_eQwRUm0OzqIoGcU8xwyLzezFU_-h0kqi8G7yEk_fSnbIV8pOzfutFAgMBAAECggEBAJQ7lHQskvU6OfDhMhLAJsTEQO88iKI7UsnAQj8CnPgcWPTFKz1sRa5otUpN_Jl-NpgGy2vzjgjk3l-SAlcVXQNYbE6RXtdwhPAxJCs3ttuyi0GcX3MYXAwlddYfdaarpLgpkrfEDcaUK3tND1tjhsv0FfzsMMXPRI7GUR0DcgItMrVCx1vt7siZa6gfh1USyKFxIEnKFiEpMTQMgDdXYpjquVzFxyv1OhaQUxeE5AN4nNYFW7v9TUAZboHZqlBT1jUR_K1a0zuAliD6Ks6m0ipjQBRJSgKJa9BI00KjeHSPFdMVCWazF7Zc02MieqOdRN_twAiu0sDiBb74EzEsjxUCgYEAwAWSY_4RvcREqtn6a5s_5UQECpqYzE9FIuKlgW5eFdHd6raaG5UXOox2M2ad-CPyjKbBMB1zXShqUXz5GQbbFwQUpNjnJhSV9Z_X6qCHOyXiyKFjZWxgP3OD-icBqhQf3Ft6GNNF_g8im6Gyu0ONUHaE1NRKyxQ3XMVfJLCFRhcCgYEAzbaaknrhF26R43mf--FXN3d86m58k5ydSAbPwLSkpTcHXHz0MWKj7lGfht-TaRGqSH47e7IRtyrMmaPEA6bWv7HuEourXTkLwNcTBXD2uwRdi0yZVo3RKL_KfvSqwkGH3RUXqpSVLnMaFc3i7Nk5pk3l6xtCJY9hPF-C9gfiTwMCgYAX2NlMV8JGSfipKzcBZB90TpUd6AMv5GxWn8UkJNvEY_LmclUDNenTmJwZWBYoOfamZxM48X9hQ2Koyhd5dzOAUT5rFpDmVsok3fwHpHYG73aRqhFZCDOPzb3HNE7tm2A1kprAOITJv4FxyIwU25fSNVXbxJ2hSNpzSAO_37g9cwKBgAzqoUv_QTDqdWiWE3CKVqKZ8xL5OwM9uzZxjwvni8r_6ItrIR4UtnxZTa33Tdc0D8AbhPqgVLJukog3GzCrgiJpNqydbnYdBdrm5j_aNvPJM2JyvdIMd4yadkmAbVRjLve3wlOonrFa8tFZqxz6Cr-hdoVLodyf4xgaWyu9lP0nAoGAXvod7KF-U7js39CjNGsX0Dog9MSPQp2iYx3PFlwXXj-XAqCHPh7ySFR4_-AfhZE_mAq1cD4Lq0mlgBr5tTJPRSfqjMIERJ1JcKOxamRufAhpCD0fvS3cs36K5_UOcLroOh1IgDPBjfRVfobDZMq5ebZV4zIRW0HVOoT-lHZPfR0"
];

impl SelfRelation {
    /// Gets a pre packaged SelfRelation. To be used for testing to avoid
    /// regenerating the key each test run. The SelfRelation will have the role
    /// Peer.
    pub fn debug_get(index: usize) -> Self {
        let item = KEYS[index];
        let bytes = general_purpose::URL_SAFE_NO_PAD.decode(item).unwrap();
        let key = RsaPrivateKey::from_pkcs8_der(&bytes).unwrap();
        Self::from_key(key, Role::Peer)
    }

    /// Gets a pre packaged SelfRelation. To be used for testing to avoid
    /// regenerating the key each test run. The SelfRelation will have the role
    /// Peripheral.
    pub fn debug_get_peripheral(index: usize) -> Self {
        let item = KEYS[index];
        let bytes = general_purpose::URL_SAFE_NO_PAD.decode(item).unwrap();
        let key = RsaPrivateKey::from_pkcs8_der(&bytes).unwrap();
        Self::from_key(key, Role::Peripheral)
    }

    /// Used to generate new keys
    pub fn debug_make() {
        let mut rng = rand::thread_rng();
        let key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate key");
        let doc = key.to_pkcs8_der().unwrap();
        let bytes = doc.as_bytes();
        let b64 = general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        println!("key = {}", b64);
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn check_debug_keys() {
        // SelfRelation::debug_make();

        for i in 0..KEYS.len() {
            let sr = SelfRelation::debug_get(i);

            let msg = b"test message";
            // println!("plain text = {:?}", msg.to_vec());
            let encrypted = sr.encrypt(&msg.to_vec());
            let decrypted = sr.decrypt(&encrypted).unwrap();
            // println!("Decrypted = {:?}", decrypted);
            assert_eq!(msg.to_vec(), decrypted);
        }
    }

    #[test]
    fn sig_len() {
        let self_rel = SelfRelation::debug_get(0);
        for x in 0..25 {
            let msg = x.to_string();
            self_rel.sign(msg.as_bytes());
        }
    }
}

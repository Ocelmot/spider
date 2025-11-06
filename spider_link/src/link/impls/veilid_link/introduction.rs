use std::{collections::VecDeque, io::Read, str::FromStr};

use veilid_core::{CryptoTyped, TypedKey};

use crate::{
    error::{ErrorKind, ProblemWrap},
    LinkError, LinkResult, RelSig, Relation, SelfRelation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VeilidIntroduction {
    from: Relation,
    sig: RelSig,
    is_listen: bool,
    dht_key: Vec<u8>,
}

impl VeilidIntroduction {
    pub fn new_listen(
        from: &SelfRelation,
        to: &Relation,
        dht_key: TypedKey,
    ) -> Self {
        // encrypt key with their Relation
        let dht_bytes = dht_key.to_string().as_bytes().to_vec();
        let dht_key = to.encrypt(&dht_bytes);

        let mut sig_bytes = Vec::with_capacity(dht_key.len()+1);
        sig_bytes.push(1); // is listen = true
        sig_bytes.extend(dht_key.iter());

        // sign encrypted key with our SelfRelation
        // println!("new_listen signature bytes {:?}", sig_bytes);
        let sig = from.sign(&sig_bytes);
        Self {
            from: from.relation.clone(),
            sig,
            is_listen: true,
            dht_key,
        }
    }

    pub fn new_connection(
        from: &SelfRelation,
        to: &Relation,
        dht_key: TypedKey,
    ) -> Self {
        // encrypt key with their Relation
        let dht_bytes = dht_key.to_string().as_bytes().to_vec();
        let dht_key = to.encrypt(&dht_bytes);

        let mut sig_bytes = Vec::with_capacity(dht_key.len()+1);
        sig_bytes.push(0); // is listen = false
        sig_bytes.extend(dht_key.iter());

        // sign encrypted key with our SelfRelation
        let sig = from.sign(&sig_bytes);
        Self {
            from: from.relation.clone(),
            sig,
            is_listen: false,
            dht_key,
        }
    }

    /// Returns the relation and dht_key, if authentic
    pub fn verify(
        self,
        self_rel: &SelfRelation,
    ) -> LinkResult<(Relation, bool, TypedKey)> {
        // Verify signature with their Relation
        let mut sig_bytes = Vec::with_capacity(self.dht_key.len()+1);
        sig_bytes.push(self.is_listen as u8);
        sig_bytes.extend(self.dht_key.iter());
        // println!("Verification signature bytes {:?}", sig_bytes);
        if !self.from.verify(&sig_bytes, &self.sig) {
            return Err(LinkError::new()
                .problem(ErrorKind::Authentication)
                .msg("Could not verify signature"));
        }

        // Get the is listen boolean
        let is_listen = self.is_listen;

        // Decrypt dht_key with our SelfRelation
        let dht_key = self_rel
            .decrypt(&self.dht_key)
            .wrap_problem_msg(ErrorKind::Authentication, "Could not decrypt dht_key")?;

        let dht_key = String::from_utf8(dht_key)
            .wrap_problem_msg(ErrorKind::Deserialization, "Could not decode dht_key")?;

        // deserialize
        let dht_key = CryptoTyped::from_str(&dht_key)
            .wrap_problem_msg(ErrorKind::Deserialization, "Could not deserialize dht_key")?;

        Ok((self.from, is_listen, dht_key))
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        ret.extend_from_slice(&self.from.serialize());
        ret.extend_from_slice(&self.sig);
        if self.is_listen {
            ret.push(1);
        }else{
            ret.push(0);
        }
        ret.extend_from_slice(&self.dht_key);
        ret
    }

    pub fn deserialize(mut data: VecDeque<u8>) -> LinkResult<Self> {
        let from = Relation::deserialize(&mut data)?;

        let mut sig = [0u8; 256];
        data.read_exact(&mut sig)
            .wrap_problem(ErrorKind::Deserialization)?;

        let is_listen = data.pop_front().wrap_problem(ErrorKind::Deserialization)?;
        let is_listen = is_listen != 0;

        let mut dht_key = Vec::new();
        data.read_to_end(&mut dht_key).wrap_problem_msg(
            ErrorKind::Deserialization,
            "Too few bytes to deserialize dht_key",
        )?;

        Ok(Self {
            from,
            sig,
            is_listen,
            dht_key,
        })
    }
}

#[cfg(test)]
mod tests {

    use rand::{random, thread_rng, Rng, RngCore};
    use veilid_core::{CryptoKey, FourCC};

    use crate::SelfRelation;

    use super::*;

    #[test]
    fn listen_introduction_round_trip() {
        let from = SelfRelation::debug_get(0);
        let to = SelfRelation::debug_get(1);

        let key = CryptoKey::new(random());
        let dht_key = CryptoTyped::new(FourCC::default(), key);

        let intro = VeilidIntroduction::new_listen(&from, &to.relation, dht_key);

        let serialized = intro.clone().serialize();
        let serialized = VecDeque::from(serialized);

        let deserialized = VeilidIntroduction::deserialize(serialized).expect("should deserialize");

        assert_eq!(intro, deserialized);
    }

    #[test]
    fn connection_introduction_round_trip() {
        let from = SelfRelation::debug_get(0);
        let to = SelfRelation::debug_get(1);

        let key = CryptoKey::new(random());
        let dht_key = CryptoTyped::new(FourCC::default(), key);

        let intro = VeilidIntroduction::new_connection(&from, &to.relation, dht_key);

        let serialized = intro.clone().serialize();
        let serialized = VecDeque::from(serialized);

        let deserialized = VeilidIntroduction::deserialize(serialized).expect("should deserialize");

        assert_eq!(intro, deserialized);
    }

    #[test]
    fn listen_introduction_verification() {
        let sender = SelfRelation::debug_get(0);
        let receiver = SelfRelation::debug_get(1);

        // Sender
        let key = CryptoKey::new(random());
        let dht_key = CryptoTyped::new(FourCC::default(), key);

        let intro = VeilidIntroduction::new_listen(&sender, &receiver.relation, dht_key);

        // Receiver
        let (verified_rel, verified_is_listen, verified_key) = intro.verify(&receiver).unwrap();

        assert_eq!(sender.relation, verified_rel);
        assert_eq!(true, verified_is_listen);
        assert_eq!(dht_key, verified_key);
    }

    #[test]
    fn connection_introduction_verification() {
        let sender = SelfRelation::debug_get(0);
        let receiver = SelfRelation::debug_get(1);

        // Sender
        let key = CryptoKey::new(random());
        let dht_key = CryptoTyped::new(FourCC::default(), key);

        let intro = VeilidIntroduction::new_connection(&sender, &receiver.relation, dht_key);

        // Receiver
        let (verified_rel, verified_is_listen, verified_key) = intro.verify(&receiver).unwrap();

        assert_eq!(sender.relation, verified_rel);
        assert_eq!(false, verified_is_listen);
        assert_eq!(dht_key, verified_key);
    }

    #[test]
    fn introduction_verification_sig_failure() {
        let sender = SelfRelation::debug_get(0);
        let receiver = SelfRelation::debug_get(1);

        // Sender
        let key = CryptoKey::new(random());
        let dht_key = CryptoTyped::new(FourCC::default(), key);

        let mut intro = VeilidIntroduction::new_listen(&sender, &receiver.relation, dht_key);
        // introduce an error
        thread_rng().fill(&mut intro.sig);

        // Receiver
        intro
            .verify(&receiver)
            .expect_err("a corrupted sig should cause verify to fail");
    }

    #[test]
    fn introduction_verification_key_failure() {
        let sender = SelfRelation::debug_get(0);
        let receiver = SelfRelation::debug_get(1);

        // Sender
        let key = CryptoKey::new(random());
        let dht_key = CryptoTyped::new(FourCC::default(), key);

        let mut intro = VeilidIntroduction::new_listen(&sender, &receiver.relation, dht_key);
        // introduce an error
        thread_rng().fill_bytes(&mut intro.dht_key);

        // Receiver
        intro
            .verify(&receiver)
            .expect_err("a corrupted sig should cause verify to fail");
    }
}

use std::{
    cmp::Ordering,
    collections::HashSet,
};

use num_bigint::BigUint;
use rsa::{
    pkcs1v15::{Signature, SigningKey, VerifyingKey},
    sha2::Sha256,
    signature::{SignatureEncoding, Signer, Verifier},
};
use serde::{Deserialize, Serialize};

use crate::{SelfRelation, SpiderId2048};

use super::ProposalAction;

/// An Id to uniquely identify a change.
pub type ChangeId = BigUint;

/// An acknowledgement of a propagating change. to the groups being propagated,
/// but not yet synchronized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChangeAck {
    /// The id of the announced change.
    id: ChangeId,
    /// The sequence number of the change, should it be accepted.
    with_seq_num: u64,
    /// The member that signed the ack.
    signatory: SpiderId2048,
    /// The signature of the node that announced the change.
    signature: Vec<u8>,
}

impl ChangeAck {
    /// Create an acknowledgement of a change with the given id and seq_num,
    /// in the given sequence. Requires the self relation to sign the data
    pub fn acknowledge(us: SelfRelation, id: ChangeId, seq_num: u64) -> Self {
        let key = us.private_key();
        let signing_key = SigningKey::<Sha256>::new(key);
        let data: Vec<u8> = [id.to_bytes_be(), seq_num.to_be_bytes().into()].concat();
        let signature = signing_key.sign(data.as_ref()).to_vec();

        Self {
            id,
            with_seq_num: seq_num,
            signatory: us.relation.id.clone(),
            signature,
        }
    }

    /// Returns a reference to the Ack's [ChangeId]
    pub fn id(&self) -> &ChangeId {
        &self.id
    }

    /// Returns the sequence number this ack recognizes for this change.
    pub fn seq_num(&self) -> u64 {
        self.with_seq_num
    }

    /// Returns a reference to the id of the signatory of this change.
    pub fn signatory(&self) -> &SpiderId2048{
        &self.signatory
    }

    /// Verifies the acknowlegement came from a member of the group
    pub fn verify(&self, members: &HashSet<SpiderId2048>) -> bool {
        if !members.contains(&self.signatory) {
            return false;
        }
        match self.signatory.as_pub_key() {
            Ok(key) => {
                let verifying_key = VerifyingKey::<Sha256>::new(key);
                let data: Vec<u8> = [
                    self.id.to_bytes_be(),
                    self.with_seq_num.to_be_bytes().into(),
                ]
                .concat();

                match Signature::try_from(self.signature.as_slice()) {
                    Ok(sig) => verifying_key.verify(&data, &sig).is_ok(),
                    Err(_) => false,
                }
            }
            Err(_) => return false,
        }
    }
}

/// The announcement of a new change. to the groups being propagated,
/// but not yet synchronized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeAnnounce {
    /// The id/signature of the announced change.
    sig: ChangeId,
    /// The id of the node to announce and sign the change.
    signatory: SpiderId2048,

    /// The sequence number of the change when it is accepted,
    /// 0 if it is not accepted.
    seq_num: u64,

    /// The announced [ProposalAction].
    proposal: ProposalAction,
}

impl ChangeAnnounce {
    /// Announce a new change with the given proposal action.
    /// This does not include an ack from the announcing node.
    pub fn announce_new(us: SelfRelation, pa: ProposalAction) -> Self {
        let key = us.private_key();
        let signing_key = SigningKey::<Sha256>::new(key);
        let data = pa.to_string();
        let sig = signing_key.sign(data.as_bytes()).to_vec();
        let sig = BigUint::from_bytes_be(&sig);
        Self {
            sig,
            signatory: us.relation.id.clone(),
            seq_num: 0,
            proposal: pa,
        }
    }

    /// Returns a reference to the id of this change.
    pub fn id(&self) -> &ChangeId {
        &self.sig
    }

    /// Returns a reference to the sequence number of this change.
    /// If the sequence number is 0, the change has not yet been accepted.
    pub fn seq_num(&self) -> u64 {
        self.seq_num
    }

    /// Returns a reference to the signatory of this ChangeAnnounce
    pub fn signatory(&self) -> &SpiderId2048 {
        &self.signatory
    }

    /// Validates the ChangeAnnounce with a set of valid members of the group.
    /// [ChangeAck]s from members not in the groups will be removed.
    /// If the signature of the ChangeAnnounce itself is not in the member set,
    /// the function will return false.
    /// If the ChangeAnnounce is not accepted, it will tally the ChangeAcks
    /// to try to determine its acceptance.
    pub fn validate_with(&self, members: &HashSet<SpiderId2048>) -> bool {
        // if the signatory is not in the group, this ChangeAnnounce is not valid.
        if !members.contains(&self.signatory) {
            return false;
        }

        // if the id/signature is invalid
        let key = self.signatory.as_pub_key().unwrap();
        let verifying_key = VerifyingKey::<Sha256>::new(key);
        let data = self.proposal.to_string();

        match Signature::try_from(self.sig.to_bytes_be().as_slice()) {
            Ok(sig) => {
                if verifying_key.verify(data.as_bytes(), &sig).is_err() {
                    return false;
                }
            }
            Err(_) => {
                // invalid signature format
                return false;
            }
        }

        // Check that the acks are valid, and re-tally the seq_num.
        // self.validate_acceptance_with(members);
        true
    }

    // /// Calculates the acceptance of the ChangeAnnounce from its set of
    // /// [ChangeAck]s.
    // pub fn validate_acceptance_with(&mut self, members: &HashSet<SpiderId2048>) {
    //     let mut seq_map: HashMap<u64, usize> = HashMap::new();
    //     let mut voted = HashSet::new();
    //     self.acks.retain(|ack| {
    //         if ack.verify(members) {
    //             if voted.contains(&ack.signatory) {
    //                 return true;
    //             } else {
    //                 voted.insert(ack.signatory.clone());
    //             }

    //             match seq_map.get_mut(&ack.seq_num()) {
    //                 Some(val) => {
    //                     *val = *val + 1;
    //                 }
    //                 None => {
    //                     seq_map.insert(ack.seq_num(), 1);
    //                 }
    //             }
    //             true
    //         } else {
    //             false
    //         }
    //     });
    //     let mut max_seq = 0;
    //     let mut max_count = 0;
    //     for (seq, count) in seq_map {
    //         if count > max_count {
    //             max_seq = seq;
    //             max_count = count;
    //         }
    //     }

    //     if max_count as f32 > (members.len() as f32 * 0.75) {
    //         self.seq_num = max_seq
    //     } else {
    //         self.seq_num = 0;
    //     }
    // }

    // /// Merges the [ChangeAck]s from an other ChangeAnnounce.
    // /// If the ids of the other ChangeAnnounce does not match, this will return
    // /// false and do nothing.
    // /// Also returns false if all of the new information was already known.
    // /// Otherwise, it will take the ChangeAcks from other and add them to its
    // /// own set of acks. This will leave the other ChangeAnnounce empty of acks.
    // pub fn merge_from(&mut self, other: &mut ChangeAnnounce) -> bool {
    //     if self.sig != other.sig {
    //         return false;
    //     }
    //     let mut found_new = false;
    //     let other_acks = mem::take(&mut other.acks);
    //     for ack in other_acks {
    //         if self.acks.insert(ack) {
    //             found_new = true;
    //         }
    //     }
    //     found_new
    // }

    // /// Returns an iterator over the set of acks
    // pub fn acks(&self) -> Iter<'_, ChangeAck> {
    //     self.acks.iter()
    // }

    // /// Adds an ack to this change, does not recalculate the accepeted
    // /// sequence number, since that requires a list of members.
    // pub fn add_ack(&mut self, ack: ChangeAck) {
    //     self.acks.insert(ack);
    // }

    // /// Remove all acks less than or equal to the most recent accepted change
    // /// sequence number. Does not recalculate if this ack should be accepted.
    // pub fn clean_acks(&mut self, most_recent_accepted_seq_num: u64) {
    //     self.acks
    //         .retain(|e| e.with_seq_num > most_recent_accepted_seq_num);
    // }

    /// Returns a reference to the [ProposalAction] this change contains.
    pub fn proposal(&self) -> &ProposalAction {
        &self.proposal
    }
}

impl PartialEq for ChangeAnnounce {
    fn eq(&self, other: &Self) -> bool {
        self.sig == other.sig && self.seq_num == other.seq_num
    }
}

impl PartialOrd for ChangeAnnounce {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(if (self.seq_num == 0) == (other.seq_num == 0) {
            // either both sequence numbers are 0 or not 0
            if self.seq_num == 0 {
                // both sequence numbers are 0 (not committed)
                self.sig.cmp(&other.sig)
            } else {
                // both sequence numbers are non-zero (committed)
                self.seq_num.cmp(&other.seq_num)
            }
        } else {
            // one sequence number is 0, the other is not
            if self.seq_num == 0 {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        })
    }
}

impl Eq for ChangeAnnounce {}

impl Ord for ChangeAnnounce {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

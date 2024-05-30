use serde::{Deserialize, Serialize};
use veilid_core::{CryptoKey, CryptoTyped};

use crate::{Relation, SelfRelation};

use super::Message;

/// Represents a message sent via the Veilid subsystem.
/// These messages are not sent through the usual Link system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VeilidMessage {
    /// Used to finish establishing a connection that was started by recieving
    /// an invite message.
    CompleteInvite {
        /// The [Relation] of the recipient of the invite
        rel: Relation,
        /// The public key of the recipient
        veilid_key: CryptoKey,
        /// The key of the DHT entry location
        dht_key: CryptoTyped<CryptoKey>,
        /// Signature to verify the Veilid information with the [Relation]
        signature: Vec<u8>,
    },

    /// Send a [Message] through the Veilid network
    Message(Message),
}

impl VeilidMessage {
    /// Prepares a [VeilidMessage::CompleteInvite] message to be set via Veilid.
    pub fn complete_invite(us: &SelfRelation, veilid_key: CryptoKey, dht_key: CryptoTyped<CryptoKey>) -> Self {
        let data = [
            veilid_key.as_slice(),
            &dht_key.to_string().into_bytes(),
        ].concat();
        let signature = us.sign(data);

        Self::CompleteInvite {
            rel: us.relation.clone(),
            veilid_key,
            dht_key,
            signature,
        }
    }

    /// Verifies a [VeilidMessage::CompleteInvite] message.
    /// If the [VeilidMessage] is not a CompleteInvite varient, it returns false.
    /// Returns if the CompleteInvite was verified correctly.
    pub fn verify_complete_invite(&self) -> bool {
        match self {
            VeilidMessage::CompleteInvite { rel, veilid_key, dht_key, signature } => {
                let data = [
                    veilid_key.as_slice(),
                    &dht_key.to_string().into_bytes(),
                ].concat();
                rel.verify(&data, signature)
            },
            VeilidMessage::Message(_) => false,
        }
    }
}

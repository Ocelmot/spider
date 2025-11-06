use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use tracing::debug;
use veilid_core::TypedKey;

use crate::{Relation, SelfRelation};

use super::Message;

/// The VeilidFrame wraps a [VeilidMessage], adding a [Relation] and a signature.
/// The relation can be used to verify the signature of the contained data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VeilidFrame {
    rel: Relation,
    data: Vec<u8>,
    sig: Vec<u8>,
    index: usize,
    count: usize,
}

impl VeilidFrame {
    // Constructors

    /// Create a new VeilidFrame by passing the data bytes directly
    pub fn new_raw(us: &SelfRelation, data: Vec<u8>) -> Vec<Self> {
        let sig = us.sign(&data);
        let  ret = Self {
            rel: us.relation.clone(),
            data,
            sig: sig.to_vec(),
            index: 0,
            count: 1,
        };
        ret.split()
    }

    /// Create a new VeilidFrame by passing a [VeilidMessage] to be
    /// serialized and be used as the frame's data
    pub fn new_msg(us: &SelfRelation, msg: &VeilidMessage) -> Vec<Self> {
        let data = serde_cbor::to_vec(&msg).unwrap();
        Self::new_raw(us, data)
    }

    /// Create a new VeilidFrame by passing the parameters needed to create a
    /// [VeilidMessage::CompleteInvite] which is then serialized into the frame's data
    pub fn new_complete_invite(us: &SelfRelation, dht_key: TypedKey, code: u64) -> Vec<Self> {
        let msg = VeilidMessage::CompleteInvite { dht_key, code };
        let data = serde_cbor::to_vec(&msg).unwrap();
        Self::new_raw(us, data)
    }

    /// Create a new VeilidFrame by passing a [Message] to be wrapped with a
    /// [VeilidMessage::Message] which is then serialized into the frame's data
    pub fn new_wrapped_msg(us: &SelfRelation, msg: Message) -> Vec<Self> {
        let msg = VeilidMessage::Message(msg);
        let data = serde_cbor::to_vec(&msg).unwrap();
        Self::new_raw(us, data)
    }

    /// Split this frame into frames that can fit through the Veilid protocol.
    fn split(mut self) -> Vec<Self> {
        let mut data = self.data.as_mut_slice();
        let mut ret = Vec::new();
        let mut index = 0;
        let limit = 5000;
        let count = (data.len() / limit) + 1;
        while data.len() > limit {
            let (head, tail) = data.split_at_mut(limit);
            data = tail;
            let chunk = Self {
                rel: self.rel.clone(),
                data: head.to_vec(),
                sig: self.sig.clone(),
                index,
                count,
            };
            index += 1;
            ret.push(chunk);
        }
        // add remaining data
        ret.push(Self {
            rel: self.rel,
            data: data.to_vec(),
            sig: self.sig,
            index,
            count,
        });

        ret
    }

    // Accessors
    /// Returns a reference the the relation this frame was sent by
    pub fn rel(&self) -> &Relation {
        &self.rel
    }

    /// gets the index of this frame in the sequence
    pub fn index(&self) -> usize {
        self.index
    }

    /// gets the count of the number of frames in this sequence
    pub fn count(&self) -> usize {
        self.count
    }

    // Misc functions

    /// Use the frame's relation and signature to verify the frame's data
    pub fn verify(&self) -> bool {
        self.rel.verify(&self.data, &self.sig)
    }

    /// Deserialize the frame's data into a [VeilidMessage] for consumption
    pub fn get_msg(&self) -> Option<VeilidMessage> {
        serde_cbor::from_slice(&self.data).ok()
    }

    /// returns the length of the data in this frame.
    pub fn data_len(&self) -> usize {
        self.data.len()
    }
}

/// This manages incoming veilid frames and if they are split, recombines them
/// into a Message.
pub struct FrameManager{
    rel: Relation,

    /// Map from signature to a map from indices to frames
    frames: HashMap<Vec<u8>, BTreeMap<usize, VeilidFrame>>,
}

impl FrameManager {
    /// Creates a new FrameManager with a [Relation]. This Relation is used to 
    /// verify incoming frames and messages.
    pub fn new(rel: Relation) ->Self{
        Self { rel, frames: HashMap::new() }
    }

    /// Add an incoming VeilidFrame, returns a [Message] if this frame completed
    /// the pending message. None otherwise.
    pub fn add_frame(&mut self, frame: VeilidFrame) -> Option<VeilidMessage> {
        // Ensure this frame is for this relation
        if frame.rel != self.rel {
            debug!("New frame not addressed to this node");
            return None;
        }

        // There are no pending frames, incoming frame is either
        // standalone or starts a new sequence.
        if frame.count == 1 {
            // standalone frame
            if frame.verify() {
                return frame.get_msg();
            } else {
                return None;
            }
        }

        debug!("adding frame with index {} and count {}", frame.index, frame.count);

        let sequence_entry = self.frames.entry(frame.sig.clone());
        let seq_map = sequence_entry.or_default();


        debug!("frame accepted into sequence with index: {} and limit: {}", frame.index, frame.count);
        seq_map.insert(frame.index, frame.clone());


        // if this frame's index equals its count, it is the last frame and can be condensed and returned.
        if seq_map.len() == frame.count {
            let mut data = Vec::new();
            for (_, frame) in self.frames.remove(&frame.sig).unwrap() {
                data.push(frame.data);
            }
            let data = data.concat();
            let frame = VeilidFrame{
                rel: frame.rel,
                data,
                sig: frame.sig,
                index: 0,
                count: 1,
            };

            if frame.verify() {
                debug!("frame verified!");
                frame.get_msg()
            }else{
                debug!("frame verification failed");
                None
            }
        }else {
            None
        }
    }
}


/// Represents a message sent via the Veilid subsystem.
/// These messages are not sent through the usual Link system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VeilidMessage {
    /// Used to finish establishing a connection that was started by receiving
    /// an invite message.
    CompleteInvite {
        /// The key of the DHT entry location
        dht_key: TypedKey,
        /// A one time use code to be allowed to connect.
        code: u64,
    },

    /// Send a [Message] through the Veilid network
    Message(Message),
}

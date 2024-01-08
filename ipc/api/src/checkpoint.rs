// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
//! Cross network messages related struct and utility functions.

use crate::cross::CrossMsg;
use crate::subnet_id::SubnetID;
use cid::multihash::Code;
use cid::multihash::MultihashDigest;
use cid::Cid;
use ethers::utils::hex;
use fvm_ipld_encoding::DAG_CBOR;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

lazy_static! {
    // Default CID used for the genesis checkpoint. Using
    // TCid::default() leads to corrupting the fvm datastore
    // for storing the cid of an inaccessible HAMT.
    pub static ref CHECKPOINT_GENESIS_CID: Cid =
        Cid::new_v1(DAG_CBOR, Code::Blake2b256.digest("genesis".as_bytes()));
}

pub type Signature = Vec<u8>;

/// The event emitted
#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct QuorumReachedEvent {
    pub obj_kind: u8,
    pub height: ChainEpoch,
    /// The checkpoint hash
    pub obj_hash: Vec<u8>,
    pub quorum_weight: TokenAmount,
}

impl Display for QuorumReachedEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "QuorumReachedEvent<height: {}, checkpoint: {}, quorum_weight: {}>",
            self.height,
            hex::encode(&self.obj_hash),
            self.quorum_weight
        )
    }
}

/// The collection of items for the bottom up checkpoint submission
#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct BottomUpCheckpointBundle {
    pub checkpoint: BottomUpCheckpoint,
    /// The list of signatures that have signed the checkpoint hash
    pub signatures: Vec<Signature>,
    /// The list of addresses that have signed the checkpoint hash
    pub signatories: Vec<Address>,
    /// The list of cross network messages
    pub cross_msgs: Vec<CrossMsg>,
}

/// The collection of items for the bottom up checkpoint submission
#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct BottomUpMsgBatch {
    /// Child subnet ID, for replay protection from other subnets where the exact same validators operate
    pub subnet_id: SubnetID,
    /// The height of the child subnet at which the batch was cut
    pub block_height: ChainEpoch,
    /// Batch of messages to execute
    pub msgs: Vec<CrossMsg>,
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct BottomUpCheckpoint {
    /// Child subnet ID, for replay protection from other subnets where the exact same validators operate.
    /// Alternatively it can be appended to the hash before signing, similar to how we use the chain ID.
    pub subnet_id: SubnetID,
    /// The height of the child subnet at which this checkpoint was cut.
    /// Has to follow the previous checkpoint by checkpoint period.
    pub block_height: ChainEpoch,
    /// The hash of the block.
    pub block_hash: Vec<u8>,
    /// The number of the membership (validator set) which is going to sign the next checkpoint.
    /// This one expected to be signed by the validators from the membership reported in the previous checkpoint.
    /// 0 could mean "no change".
    pub next_configuration_number: u64,
}

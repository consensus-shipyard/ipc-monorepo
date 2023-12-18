// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

/// This type definitions are borrowed from
/// https://github.com/consensus-shipyard/ipc-actors/blob/main/subnet-actor/src/types.rs
/// to ensure that they are in sync in this project.
/// However, we should either deprecate the native actors, or make
/// them use the types from this sdk directly.
use crate::subnet_id::SubnetID;
use fvm_ipld_encoding::repr::*;
use fvm_ipld_encoding::tuple::{Deserialize_tuple, Serialize_tuple};
use fvm_shared::{address::Address, clock::ChainEpoch, econ::TokenAmount};

/// ID used in the builtin-actors bundle manifest
pub const MANIFEST_ID: &str = "ipc_subnet_actor";

#[derive(Clone, Debug, Serialize_tuple, Deserialize_tuple, PartialEq, Eq)]
pub struct ConstructParams {
    pub parent: SubnetID,
    pub ipc_gateway_addr: Address,
    pub consensus: ConsensusType,
    pub min_validator_stake: TokenAmount,
    pub min_validators: u64,
    pub bottomup_check_period: ChainEpoch,
    pub active_validators_limit: u16,
    pub min_cross_msg_fee: TokenAmount,
    pub permissioned: bool,
}

/// Consensus types supported by hierarchical consensus
#[derive(PartialEq, Eq, Clone, Copy, Debug, Deserialize_repr, Serialize_repr)]
#[repr(u64)]
pub enum ConsensusType {
    Fendermint,
}

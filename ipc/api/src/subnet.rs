// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

/// This type definitions are borrowed from
/// https://github.com/consensus-shipyard/ipc-actors/blob/main/subnet-actor/src/types.rs
/// to ensure that they are in sync in this project.
/// However, we should either deprecate the native actors, or make
/// them use the types from this sdk directly.
use crate::subnet_id::SubnetID;
use anyhow::anyhow;
use fvm_ipld_encoding::repr::*;
use fvm_shared::{address::Address, clock::ChainEpoch, econ::TokenAmount};
use serde::{Deserialize, Serialize};

/// ID used in the builtin-actors bundle manifest
pub const MANIFEST_ID: &str = "ipc_subnet_actor";

/// Determines the permission mode for validators.
#[repr(u8)]
#[derive(Debug, Clone, Serialize_repr, Deserialize_repr, PartialEq, Eq)]
pub enum PermissionMode {
    /// Validator power is determined by the collateral staked
    Collateral,
    /// Validator power is assigned by the owner of the subnet
    Federated,
    /// Validator power is determined by the initial collateral staked and does not change anymore
    Static,
}

/// Defines the supply source of a subnet on its parent subnet.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupplySource {
    /// The kind of supply.
    pub kind: SupplyKind,
    /// The address of the ERC20 token if that supply kind is selected.
    pub token_address: Option<Address>,
}

/// Determines the type of supply used by the subnet.
#[repr(u8)]
#[derive(Debug, Clone, Serialize_repr, Deserialize_repr, PartialEq, Eq)]
pub enum SupplyKind {
    Native,
    ERC20,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConstructParams {
    pub parent: SubnetID,
    pub ipc_gateway_addr: Address,
    pub consensus: ConsensusType,
    pub min_validator_stake: TokenAmount,
    pub min_validators: u64,
    pub bottomup_check_period: ChainEpoch,
    pub active_validators_limit: u16,
    pub min_cross_msg_fee: TokenAmount,
    pub permission_mode: PermissionMode,
    pub supply_source: SupplySource,
}

/// Consensus types supported by hierarchical consensus
#[derive(PartialEq, Eq, Clone, Copy, Debug, Deserialize_repr, Serialize_repr)]
#[repr(u64)]
pub enum ConsensusType {
    Fendermint,
}

impl TryFrom<u8> for PermissionMode {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => PermissionMode::Collateral,
            1 => PermissionMode::Federated,
            2 => PermissionMode::Static,
            _ => return Err(anyhow!("unknown permission mode {value}")),
        })
    }
}

impl TryFrom<u8> for SupplyKind {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => SupplyKind::Native,
            1 => SupplyKind::ERC20,
            _ => return Err(anyhow!("unknown supply kind {value}")),
        })
    }
}

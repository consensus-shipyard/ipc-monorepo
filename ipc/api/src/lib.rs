// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
use fvm_shared::{address::Address, econ::TokenAmount};
use primitives::EthAddress;
use std::str::FromStr;

pub mod address;
pub mod checkpoint;
pub mod cross;
pub mod error;
pub mod gateway;
#[cfg(feature = "fil-actor")]
mod runtime;
pub mod subnet;
pub mod subnet_id;
pub mod validator;

pub mod evm;
pub mod staking;

/// Converts an ethers::U256 TokenAmount into a FIL amount.
pub fn eth_to_fil_amount(amount: &ethers::types::U256) -> anyhow::Result<TokenAmount> {
    let v = fvm_shared::bigint::BigInt::from_str(&amount.to_string())?;
    Ok(TokenAmount::from_atto(v))
}

pub fn ethers_address_to_fil_address(addr: &ethers::types::Address) -> anyhow::Result<Address> {
    let raw_addr = format!("{addr:?}");
    log::debug!("raw evm subnet addr: {raw_addr:}");

    let eth_addr = EthAddress::from_str(&raw_addr)?;
    Ok(Address::from(eth_addr))
}

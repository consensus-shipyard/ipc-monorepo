// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

use fvm_shared::address::{set_current_network, Network};
use num_traits::cast::FromPrimitive;

pub const DEFAULT_ROOT: &str = "/r31415926";

/// Sets the type of network from an environmental variable.
/// This is key to set the right network prefixes on string
/// representation of addresses.
pub fn set_network_from_env() {
    let network_raw: u8 = std::env::var("LOTUS_NETWORK")
        // default to testnet
        .unwrap_or_else(|_| String::from("1"))
        .parse()
        .unwrap();
    let network = Network::from_u8(network_raw).unwrap();
    set_current_network(network);
}

pub mod infra;

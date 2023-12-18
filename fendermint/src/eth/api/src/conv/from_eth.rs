// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

//! Helper methods to convert between Ethereum and FVM data formats.

use ethers_core::types::transaction::eip2718::TypedTransaction;

pub use fendermint_vm_message::conv::from_eth::*;
use fvm_shared::{error::ExitCode, message::Message};

use crate::{error, JsonRpcResult};

pub fn to_fvm_message(tx: TypedTransaction, accept_legacy: bool) -> JsonRpcResult<Message> {
    match tx {
        TypedTransaction::Eip1559(ref tx) => {
            Ok(fendermint_vm_message::conv::from_eth::to_fvm_message(tx)?)
        }
        TypedTransaction::Legacy(_) if accept_legacy => {
            // legacy transactions are only accepted for gas estimation purposes
            // (when accept_legacy is explicitly set)
            // eth_sendRawTransaction should fail for legacy transactions.
            // For this purpose it os OK to not set `max_fee_per_gas` and
            // `max_priority_fee_per_gas`. Legacy transactions don't include
            // that information
            Ok(fendermint_vm_message::conv::from_eth::to_fvm_message(
                &tx.into(),
            )?)
        }
        TypedTransaction::Legacy(_) | TypedTransaction::Eip2930(_) => error(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "unexpected transaction type",
        ),
    }
}

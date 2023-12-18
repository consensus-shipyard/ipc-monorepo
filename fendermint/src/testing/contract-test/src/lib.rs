// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::{anyhow, Context};
use std::sync::Arc;

use fendermint_vm_genesis::Genesis;
use fendermint_vm_interpreter::{
    fvm::{
        bundle::{bundle_path, contracts_path},
        state::{FvmExecState, FvmGenesisState},
        store::memory::MemoryBlockstore,
        FvmGenesisOutput, FvmMessageInterpreter,
    },
    GenesisInterpreter,
};
use fvm::engine::MultiEngine;

pub mod ipc;

pub async fn init_exec_state(
    multi_engine: Arc<MultiEngine>,
    genesis: Genesis,
) -> anyhow::Result<(FvmExecState<MemoryBlockstore>, FvmGenesisOutput)> {
    let bundle = std::fs::read(bundle_path()).context("failed to read bundle")?;
    let store = MemoryBlockstore::new();

    let state = FvmGenesisState::new(store, multi_engine, &bundle)
        .await
        .context("failed to create state")?;

    let (client, _) =
        tendermint_rpc::MockClient::new(tendermint_rpc::MockRequestMethodMatcher::default());

    let interpreter = FvmMessageInterpreter::new(client, None, contracts_path(), 1.05, 1.05, false);

    let (state, out) = interpreter
        .init(state, genesis)
        .await
        .context("failed to create actors")?;

    let state = state
        .into_exec_state()
        .map_err(|_| anyhow!("should be in exec stage"))?;

    Ok((state, out))
}

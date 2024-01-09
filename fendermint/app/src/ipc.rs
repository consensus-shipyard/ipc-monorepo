// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
//! IPC related execution

use crate::app::{AppState, AppStoreKey};
use crate::{App, BlockHeight};
use fendermint_storage::{Codec, Encode, KVReadable, KVStore, KVWritable};
use fendermint_vm_interpreter::fvm::state::ipc::GatewayCaller;
use fendermint_vm_interpreter::fvm::state::FvmStateParams;
use fendermint_vm_interpreter::fvm::store::ReadOnlyBlockstore;
use fendermint_vm_topdown::sync::ParentFinalityStateQuery;
use fendermint_vm_topdown::IPCParentFinality;
use fvm_ipld_blockstore::Blockstore;
use std::sync::Arc;

/// Queries the LATEST COMMITTED parent finality from the storage
pub struct AppParentFinalityQuery<DB, SS, S, I>
where
    SS: Blockstore + 'static,
    S: KVStore,
{
    /// The app to get state
    app: App<DB, SS, S, I>,
    gateway_caller: GatewayCaller<ReadOnlyBlockstore<Arc<SS>>>,
}

impl<DB, SS, S, I> AppParentFinalityQuery<DB, SS, S, I>
where
    S: KVStore
        + Codec<AppState>
        + Encode<AppStoreKey>
        + Encode<BlockHeight>
        + Codec<FvmStateParams>,
    DB: KVWritable<S> + KVReadable<S> + 'static + Clone,
    SS: Blockstore + 'static + Clone,
{
    pub fn new(app: App<DB, SS, S, I>) -> Self {
        Self {
            app,
            gateway_caller: GatewayCaller::default(),
        }
    }
}

impl<DB, SS, S, I> ParentFinalityStateQuery for AppParentFinalityQuery<DB, SS, S, I>
where
    S: KVStore
        + Codec<AppState>
        + Encode<AppStoreKey>
        + Encode<BlockHeight>
        + Codec<FvmStateParams>,
    DB: KVWritable<S> + KVReadable<S> + 'static + Clone,
    SS: Blockstore + 'static + Clone,
{
    fn get_latest_committed_finality(&self) -> anyhow::Result<Option<IPCParentFinality>> {
        let maybe_exec_state = self.app.new_read_only_exec_state()?;

        let finality = if let Some(mut exec_state) = maybe_exec_state {
            let finality = self
                .gateway_caller
                .get_latest_parent_finality(&mut exec_state)?;
            Some(finality)
        } else {
            None
        };

        Ok(finality)
    }
}

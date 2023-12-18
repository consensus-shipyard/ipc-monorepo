// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_stm::{atomically, atomically_or_err};
use async_trait::async_trait;
use cid::Cid;
use fendermint_abci::util::take_until_max_size;
use fendermint_abci::{AbciResult, Application};
use fendermint_storage::{
    Codec, Encode, KVCollection, KVRead, KVReadable, KVStore, KVWritable, KVWrite,
};
use fendermint_vm_core::Timestamp;
use fendermint_vm_genesis::{Power, Validator};
use fendermint_vm_interpreter::bytes::{
    BytesMessageApplyRes, BytesMessageCheckRes, BytesMessageQuery, BytesMessageQueryRes,
};
use fendermint_vm_interpreter::chain::{
    ChainMessageApplyRet, CheckpointPool, IllegalMessage, TopDownFinalityProvider,
};
use fendermint_vm_interpreter::fvm::state::{
    empty_state_tree, CheckStateRef, FvmExecState, FvmGenesisState, FvmQueryState, FvmStateParams,
    FvmUpdatableParams,
};
use fendermint_vm_interpreter::fvm::store::ReadOnlyBlockstore;
use fendermint_vm_interpreter::fvm::{FvmApplyRet, FvmGenesisOutput};
use fendermint_vm_interpreter::signed::InvalidSignature;
use fendermint_vm_interpreter::{
    CheckInterpreter, ExecInterpreter, GenesisInterpreter, ProposalInterpreter, QueryInterpreter,
};
use fendermint_vm_message::query::FvmQueryHeight;
use fendermint_vm_snapshot::{SnapshotClient, SnapshotError};
use fvm::engine::MultiEngine;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::chainid::ChainID;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::version::NetworkVersion;
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use tendermint::abci::request::CheckTxKind;
use tendermint::abci::{request, response};

use crate::{tmconv::*, VERSION};
use crate::{BlockHeight, APP_VERSION};

#[derive(Serialize)]
#[repr(u8)]
pub enum AppStoreKey {
    State,
}

// TODO: What range should we use for our own error codes? Should we shift FVM errors?
#[derive(Debug)]
#[repr(u32)]
pub enum AppError {
    /// Failed to deserialize the transaction.
    InvalidEncoding = 51,
    /// Failed to validate the user signature.
    InvalidSignature = 52,
    /// User sent a message they should not construct.
    IllegalMessage = 53,
    /// The genesis block hasn't been initialized yet.
    NotInitialized = 54,
}

/// The application state record we keep a history of in the database.
#[derive(Serialize, Deserialize)]
pub struct AppState {
    /// Last committed block height.
    block_height: BlockHeight,
    /// Oldest state hash height.
    oldest_state_height: BlockHeight,
    /// Last committed version of the evolving state of the FVM.
    state_params: FvmStateParams,
}

impl AppState {
    pub fn state_root(&self) -> Cid {
        self.state_params.state_root
    }

    pub fn chain_id(&self) -> ChainID {
        ChainID::from(self.state_params.chain_id)
    }

    pub fn app_hash(&self) -> tendermint::hash::AppHash {
        to_app_hash(&self.state_params)
    }

    /// The state is effective at the *next* block, that is, the effects of block N are visible in the header of block N+1,
    /// so the height of the state itself as a "post-state" is one higher than the block which we executed to create it.
    pub fn state_height(&self) -> BlockHeight {
        self.block_height + 1
    }
}

pub struct AppConfig<S: KVStore> {
    /// Namespace to store the current app state.
    pub app_namespace: S::Namespace,
    /// Namespace to store the app state history.
    pub state_hist_namespace: S::Namespace,
    /// Size of state history to keep; 0 means unlimited.
    pub state_hist_size: u64,
    /// Path to the Wasm bundle.
    ///
    /// Only loaded once during genesis; later comes from the [`StateTree`].
    pub builtin_actors_bundle: PathBuf,
}

/// Handle ABCI requests.
#[derive(Clone)]
pub struct App<DB, SS, S, I>
where
    SS: Blockstore + 'static,
    S: KVStore,
{
    /// Database backing all key-value operations.
    db: Arc<DB>,
    /// State store, backing all the smart contracts.
    ///
    /// Must be kept separate from storage that can be influenced by network operations such as Bitswap;
    /// nodes must be able to run transactions deterministically. By contrast the Bitswap store should
    /// be able to read its own storage area as well as state storage, to serve content from both.
    state_store: Arc<SS>,
    /// Wasm engine cache.
    multi_engine: Arc<MultiEngine>,
    /// Path to the Wasm bundle.
    ///
    /// Only loaded once during genesis; later comes from the [`StateTree`].
    builtin_actors_bundle: PathBuf,
    /// Namespace to store app state.
    namespace: S::Namespace,
    /// Collection of past state parameters.
    ///
    /// We store the state hash for the height of the block where it was committed,
    /// which is different from how Tendermint Core will refer to it in queries,
    /// shifted by one, because Tendermint Core will use the height where the hash
    /// *appeared*, which is in the block *after* the one which was committed.
    ///
    /// The state also contains things like timestamp and the network version,
    /// so that we can retrospectively execute FVM messages at past block heights
    /// in read-only mode.
    state_hist: KVCollection<S, BlockHeight, FvmStateParams>,
    /// Interpreter for block lifecycle events.
    interpreter: Arc<I>,
    /// CID resolution pool.
    resolve_pool: CheckpointPool,
    /// The parent finality provider for top down checkpoint
    parent_finality_provider: TopDownFinalityProvider,
    /// Interface to the snapshotter, if enabled.
    snapshots: Option<SnapshotClient>,
    /// State accumulating changes during block execution.
    exec_state: Arc<tokio::sync::Mutex<Option<FvmExecState<SS>>>>,
    /// Projected (partial) state accumulating during transaction checks.
    check_state: CheckStateRef<SS>,
    /// How much history to keep.
    ///
    /// Zero means unlimited.
    state_hist_size: u64,
}

impl<DB, SS, S, I> App<DB, SS, S, I>
where
    S: KVStore
        + Codec<AppState>
        + Encode<AppStoreKey>
        + Encode<BlockHeight>
        + Codec<FvmStateParams>,
    DB: KVWritable<S> + KVReadable<S> + Clone + 'static,
    SS: Blockstore + Clone + 'static,
{
    pub fn new(
        config: AppConfig<S>,
        db: DB,
        state_store: SS,
        interpreter: I,
        resolve_pool: CheckpointPool,
        parent_finality_provider: TopDownFinalityProvider,
        snapshots: Option<SnapshotClient>,
    ) -> Result<Self> {
        let app = Self {
            db: Arc::new(db),
            state_store: Arc::new(state_store),
            multi_engine: Arc::new(MultiEngine::new(1)),
            builtin_actors_bundle: config.builtin_actors_bundle,
            namespace: config.app_namespace,
            state_hist: KVCollection::new(config.state_hist_namespace),
            state_hist_size: config.state_hist_size,
            interpreter: Arc::new(interpreter),
            resolve_pool,
            parent_finality_provider,
            snapshots,
            exec_state: Arc::new(tokio::sync::Mutex::new(None)),
            check_state: Arc::new(tokio::sync::Mutex::new(None)),
        };
        app.init_committed_state()?;
        Ok(app)
    }
}

impl<DB, SS, S, I> App<DB, SS, S, I>
where
    S: KVStore
        + Codec<AppState>
        + Encode<AppStoreKey>
        + Encode<BlockHeight>
        + Codec<FvmStateParams>,
    DB: KVWritable<S> + KVReadable<S> + 'static + Clone,
    SS: Blockstore + 'static + Clone,
{
    /// Get an owned clone of the state store.
    fn state_store_clone(&self) -> SS {
        self.state_store.as_ref().clone()
    }

    /// Ensure the store has some initial state.
    fn init_committed_state(&self) -> Result<()> {
        if self.get_committed_state()?.is_none() {
            // We need to be careful never to run a query on this.
            let mut state_tree = empty_state_tree(self.state_store_clone())
                .context("failed to create empty state tree")?;
            let state_root = state_tree.flush()?;
            let state = AppState {
                block_height: 0,
                oldest_state_height: 0,
                state_params: FvmStateParams {
                    timestamp: Timestamp(0),
                    state_root,
                    network_version: NetworkVersion::V0,
                    base_fee: TokenAmount::zero(),
                    circ_supply: TokenAmount::zero(),
                    chain_id: 0,
                    power_scale: 0,
                },
            };
            self.set_committed_state(state)?;
        }
        Ok(())
    }

    /// Get the last committed state, if exists.
    fn get_committed_state(&self) -> Result<Option<AppState>> {
        let tx = self.db.read();
        tx.get(&self.namespace, &AppStoreKey::State)
            .context("get failed")
    }

    /// Get the last committed state; return error if it doesn't exist.
    fn committed_state(&self) -> Result<AppState> {
        match self.get_committed_state()? {
            Some(state) => Ok(state),
            None => Err(anyhow!("app state not found")),
        }
    }

    /// Set the last committed state.
    fn set_committed_state(&self, mut state: AppState) -> Result<()> {
        self.db
            .with_write(|tx| {
                // Insert latest state history point at the `block_height + 1`,
                // to be consistent with how CometBFT queries are supposed to work.
                let state_height = state.state_height();

                self.state_hist
                    .put(tx, &state_height, &state.state_params)?;

                // Prune state history.
                if self.state_hist_size > 0 && state_height >= self.state_hist_size {
                    let prune_height = state_height.saturating_sub(self.state_hist_size);
                    while state.oldest_state_height <= prune_height {
                        self.state_hist.delete(tx, &state.oldest_state_height)?;
                        state.oldest_state_height += 1;
                    }
                }

                // Update the application state.
                tx.put(&self.namespace, &AppStoreKey::State, &state)?;

                Ok(())
            })
            .context("commit failed")
    }

    /// Put the execution state during block execution. Has to be empty.
    async fn put_exec_state(&self, state: FvmExecState<SS>) {
        let mut guard = self.exec_state.lock().await;
        assert!(guard.is_none(), "exec state not empty");
        *guard = Some(state);
    }

    /// Take the execution state during block execution. Has to be non-empty.
    async fn take_exec_state(&self) -> FvmExecState<SS> {
        let mut guard = self.exec_state.lock().await;
        guard.take().expect("exec state empty")
    }

    /// Take the execution state, update it, put it back, return the output.
    async fn modify_exec_state<T, F, R>(&self, f: F) -> Result<T>
    where
        F: FnOnce((CheckpointPool, TopDownFinalityProvider, FvmExecState<SS>)) -> R,
        R: Future<
            Output = Result<(
                (CheckpointPool, TopDownFinalityProvider, FvmExecState<SS>),
                T,
            )>,
        >,
    {
        let mut guard = self.exec_state.lock().await;
        let state = guard.take().expect("exec state empty");

        let ((_pool, _provider, state), ret) = f((
            self.resolve_pool.clone(),
            self.parent_finality_provider.clone(),
            state,
        ))
        .await?;

        *guard = Some(state);

        Ok(ret)
    }

    /// Get a read only fvm execution state. This is useful to perform query commands targeting
    /// the latest state.
    pub fn new_read_only_exec_state(
        &self,
    ) -> Result<Option<FvmExecState<ReadOnlyBlockstore<Arc<SS>>>>> {
        let maybe_app_state = self.get_committed_state()?;

        Ok(if let Some(app_state) = maybe_app_state {
            let block_height = app_state.block_height;
            let state_params = app_state.state_params;

            // wait for block production
            if !Self::can_query_state(block_height, &state_params) {
                return Ok(None);
            }

            let exec_state = FvmExecState::new(
                ReadOnlyBlockstore::new(self.state_store.clone()),
                self.multi_engine.as_ref(),
                block_height as ChainEpoch,
                state_params,
            )
            .context("error creating execution state")?;

            Some(exec_state)
        } else {
            None
        })
    }

    /// Look up a past state at a particular height Tendermint Core is looking for.
    ///
    /// A height of zero means we are looking for the latest state.
    /// The genesis block state is saved under height 1.
    /// Under height 0 we saved the empty state, which we must not query,
    /// because it doesn't contain any initialized state for the actors.
    ///
    /// Returns the state params and the height of the block which committed it.
    fn state_params_at_height(
        &self,
        height: FvmQueryHeight,
    ) -> Result<(FvmStateParams, BlockHeight)> {
        if let FvmQueryHeight::Height(h) = height {
            let tx = self.db.read();
            let sh = self
                .state_hist
                .get(&tx, &h)
                .context("error looking up history")?;

            if let Some(p) = sh {
                return Ok((p, h));
            }
        }
        let state = self.committed_state()?;
        Ok((state.state_params, state.block_height))
    }

    /// Check whether the state has been initialized by genesis.
    ///
    /// We can't run queries on the initial empty state becase the actors haven't been inserted yet.
    fn can_query_state(height: BlockHeight, params: &FvmStateParams) -> bool {
        // It's really the empty state tree that would be the best indicator.
        !(height == 0 && params.timestamp.0 == 0 && params.network_version == NetworkVersion::V0)
    }
}

// NOTE: The `Application` interface doesn't allow failures at the moment. The protobuf
// of `Response` actually has an `Exception` type, so in theory we could use that, and
// Tendermint would break up the connection. However, before the response could reach it,
// the `tower-abci` library would throw an exception when it tried to convert a
// `Response::Exception` into a `ConsensusResponse` for example.
#[async_trait]
impl<DB, SS, S, I> Application for App<DB, SS, S, I>
where
    S: KVStore
        + Codec<AppState>
        + Encode<AppStoreKey>
        + Encode<BlockHeight>
        + Codec<FvmStateParams>,
    S::Namespace: Sync + Send,
    DB: KVWritable<S> + KVReadable<S> + Clone + Send + Sync + 'static,
    SS: Blockstore + Clone + Send + Sync + 'static,
    I: GenesisInterpreter<
        State = FvmGenesisState<SS>,
        Genesis = Vec<u8>,
        Output = FvmGenesisOutput,
    >,
    I: ProposalInterpreter<State = (CheckpointPool, TopDownFinalityProvider), Message = Vec<u8>>,
    I: ExecInterpreter<
        State = (CheckpointPool, TopDownFinalityProvider, FvmExecState<SS>),
        Message = Vec<u8>,
        BeginOutput = FvmApplyRet,
        DeliverOutput = BytesMessageApplyRes,
        EndOutput = Vec<Validator<Power>>,
    >,
    I: CheckInterpreter<
        State = FvmExecState<ReadOnlyBlockstore<SS>>,
        Message = Vec<u8>,
        Output = BytesMessageCheckRes,
    >,
    I: QueryInterpreter<
        State = FvmQueryState<SS>,
        Query = BytesMessageQuery,
        Output = BytesMessageQueryRes,
    >,
{
    /// Provide information about the ABCI application.
    async fn info(&self, _request: request::Info) -> AbciResult<response::Info> {
        let state = self.committed_state()?;

        let height = tendermint::block::Height::try_from(state.block_height)?;

        let info = response::Info {
            data: "fendermint".to_string(),
            version: VERSION.to_owned(),
            app_version: APP_VERSION,
            last_block_height: height,
            last_block_app_hash: state.app_hash(),
        };
        Ok(info)
    }

    /// Called once upon genesis.
    async fn init_chain(&self, request: request::InitChain) -> AbciResult<response::InitChain> {
        let bundle = &self.builtin_actors_bundle;
        let bundle = std::fs::read(bundle)
            .map_err(|e| anyhow!("failed to load bundle CAR from {bundle:?}: {e}"))?;

        let state =
            FvmGenesisState::new(self.state_store_clone(), self.multi_engine.clone(), &bundle)
                .await
                .context("failed to create genesis state")?;

        tracing::info!(
            manifest_root = format!("{}", state.manifest_data_cid),
            "pre-genesis state created"
        );

        let genesis_bytes = request.app_state_bytes.to_vec();
        let genesis_hash =
            fendermint_vm_message::cid(&genesis_bytes).context("failed to compute genesis CID")?;

        // Make it easy to spot any discrepancies between nodes.
        tracing::info!(genesis_hash = genesis_hash.to_string(), "genesis");

        let (state, out) = self
            .interpreter
            .init(state, genesis_bytes)
            .await
            .context("failed to init from genesis")?;

        let state_root = state.commit().context("failed to commit genesis state")?;
        let validators =
            to_validator_updates(out.validators).context("failed to convert validators")?;

        // Let's pretend that the genesis state is that of a fictive block at height 0.
        // The record will be stored under height 1, and the record after the application
        // of the actual block 1 will be at height 2, so they are distinct records.
        // That is despite the fact that block 1 will share the timestamp with genesis,
        // however it seems like it goes through a `prepare_proposal` phase too, which
        // suggests it could have additional transactions affecting the state hash.
        // By keeping them separate we can actually run queries at height=1 as well as height=2,
        // to see the difference between `genesis.json` only and whatever else is in block 1.
        let height: u64 = request.initial_height.into();
        // Note that setting the `initial_height` to 0 doesn't seem to have an effect.
        let height = height - 1;

        let app_state = AppState {
            block_height: height,
            oldest_state_height: height,
            state_params: FvmStateParams {
                state_root,
                timestamp: out.timestamp,
                network_version: out.network_version,
                base_fee: out.base_fee,
                circ_supply: out.circ_supply,
                chain_id: out.chain_id.into(),
                power_scale: out.power_scale,
            },
        };

        let response = response::InitChain {
            consensus_params: None,
            validators,
            app_hash: app_state.app_hash(),
        };

        tracing::info!(
            height,
            state_root = app_state.state_root().to_string(),
            app_hash = app_state.app_hash().to_string(),
            timestamp = app_state.state_params.timestamp.0,
            chain_id = app_state.state_params.chain_id,
            "init chain"
        );

        self.set_committed_state(app_state)?;

        Ok(response)
    }

    /// Query the application for data at the current or past height.
    async fn query(&self, request: request::Query) -> AbciResult<response::Query> {
        let db = self.state_store_clone();
        let height = FvmQueryHeight::from(request.height.value());
        let (state_params, block_height) = self.state_params_at_height(height)?;

        tracing::debug!(
            query_height = request.height.value(),
            block_height,
            state_root = state_params.state_root.to_string(),
            "running query"
        );

        // Don't run queries on the empty state, they won't work.
        if !Self::can_query_state(block_height, &state_params) {
            return Ok(invalid_query(
                AppError::NotInitialized,
                "The app hasn't been initialized yet.".to_owned(),
            ));
        }

        let state = FvmQueryState::new(
            db,
            self.multi_engine.clone(),
            block_height.try_into()?,
            state_params,
            self.check_state.clone(),
            height == FvmQueryHeight::Pending,
        )
        .context("error creating query state")?;

        let qry = (request.path, request.data.to_vec());

        let (_, result) = self
            .interpreter
            .query(state, qry)
            .await
            .context("error running query")?;

        let response = match result {
            Err(e) => invalid_query(AppError::InvalidEncoding, e.description),
            Ok(result) => to_query(result, block_height)?,
        };
        Ok(response)
    }

    /// Check the given transaction before putting it into the local mempool.
    async fn check_tx(&self, request: request::CheckTx) -> AbciResult<response::CheckTx> {
        // Keep the guard through the check, so there can be only one at a time.
        let mut guard = self.check_state.lock().await;

        let state = match guard.take() {
            Some(state) => state,
            None => {
                let db = self.state_store_clone();
                let state = self.committed_state()?;

                // This would create a partial state, but some client scenarios need the full one.
                // FvmCheckState::new(db, state.state_root(), state.chain_id())
                //     .context("error creating check state")?

                FvmExecState::new(
                    ReadOnlyBlockstore::new(db),
                    self.multi_engine.as_ref(),
                    state.block_height.try_into()?,
                    state.state_params,
                )
                .context("error creating check state")?
            }
        };

        let (state, result) = self
            .interpreter
            .check(
                state,
                request.tx.to_vec(),
                request.kind == CheckTxKind::Recheck,
            )
            .await
            .context("error running check")?;

        // Update the check state.
        *guard = Some(state);

        let response = match result {
            Err(e) => invalid_check_tx(AppError::InvalidEncoding, e.description),
            Ok(result) => match result {
                Err(IllegalMessage) => invalid_check_tx(AppError::IllegalMessage, "".to_owned()),
                Ok(Err(InvalidSignature(d))) => invalid_check_tx(AppError::InvalidSignature, d),
                Ok(Ok(ret)) => to_check_tx(ret),
            },
        };

        Ok(response)
    }

    /// Amend which transactions to put into the next block proposal.
    async fn prepare_proposal(
        &self,
        request: request::PrepareProposal,
    ) -> AbciResult<response::PrepareProposal> {
        tracing::debug!(
            height = request.height.value(),
            time = request.time.to_string(),
            "prepare proposal"
        );
        let txs = request.txs.into_iter().map(|tx| tx.to_vec()).collect();

        let txs = self
            .interpreter
            .prepare(
                (
                    self.resolve_pool.clone(),
                    self.parent_finality_provider.clone(),
                ),
                txs,
            )
            .await
            .context("failed to prepare proposal")?;

        let txs = txs.into_iter().map(bytes::Bytes::from).collect();
        let txs = take_until_max_size(txs, request.max_tx_bytes.try_into().unwrap());

        Ok(response::PrepareProposal { txs })
    }

    /// Inspect a proposal and decide whether to vote on it.
    async fn process_proposal(
        &self,
        request: request::ProcessProposal,
    ) -> AbciResult<response::ProcessProposal> {
        tracing::debug!(
            height = request.height.value(),
            time = request.time.to_string(),
            "process proposal"
        );
        let txs = request.txs.into_iter().map(|tx| tx.to_vec()).collect();

        let accept = self
            .interpreter
            .process(
                (
                    self.resolve_pool.clone(),
                    self.parent_finality_provider.clone(),
                ),
                txs,
            )
            .await
            .context("failed to process proposal")?;

        if accept {
            Ok(response::ProcessProposal::Accept)
        } else {
            Ok(response::ProcessProposal::Reject)
        }
    }

    /// Signals the beginning of a new block, prior to any `DeliverTx` calls.
    async fn begin_block(&self, request: request::BeginBlock) -> AbciResult<response::BeginBlock> {
        let block_height = request.header.height.into();
        let block_hash = match request.hash {
            tendermint::Hash::Sha256(h) => h,
            tendermint::Hash::None => return Err(anyhow!("empty block hash").into()),
        };

        tracing::debug!(
            height = block_height,
            app_hash = request.header.app_hash.to_string(),
            "begin block"
        );

        let db = self.state_store_clone();
        let state = self.committed_state()?;
        let mut state_params = state.state_params.clone();

        // Notify the snapshotter. We don't do this in `commit` because *this* is the height at which
        // this state has been officially associated with the application hash, which is something
        // we will receive in `offer_snapshot` and we can compare. If we did it in `commit` we'd
        // have to associate the snapshot with `block_height + 1`. But this way we also know that
        // others have agreed with our results.
        if let Some(ref snapshots) = self.snapshots {
            atomically(|| snapshots.notify(block_height as u64, state_params.clone())).await;
        }

        state_params.timestamp = to_timestamp(request.header.time);

        let state = FvmExecState::new(db, self.multi_engine.as_ref(), block_height, state_params)
            .context("error creating new state")?
            .with_block_hash(block_hash);

        tracing::debug!("initialized exec state");

        self.put_exec_state(state).await;

        let ret = self
            .modify_exec_state(|s| self.interpreter.begin(s))
            .await
            .context("begin failed")?;

        Ok(to_begin_block(ret))
    }

    /// Apply a transaction to the application's state.
    async fn deliver_tx(&self, request: request::DeliverTx) -> AbciResult<response::DeliverTx> {
        let msg = request.tx.to_vec();
        let (result, block_hash) = self
            .modify_exec_state(|s| async {
                let ((pool, provider, state), res) = self.interpreter.deliver(s, msg).await?;
                let block_hash = state.block_hash();
                Ok(((pool, provider, state), (res, block_hash)))
            })
            .await
            .context("deliver failed")?;

        let response = match result {
            Err(e) => invalid_deliver_tx(AppError::InvalidEncoding, e.description),
            Ok(ret) => match ret {
                ChainMessageApplyRet::Signed(Err(InvalidSignature(d))) => {
                    invalid_deliver_tx(AppError::InvalidSignature, d)
                }
                ChainMessageApplyRet::Signed(Ok(ret)) => {
                    to_deliver_tx(ret.fvm, ret.domain_hash, block_hash)
                }
                ChainMessageApplyRet::Ipc(ret) => to_deliver_tx(ret, None, block_hash),
            },
        };

        if response.code != 0.into() {
            tracing::info!(
                "deliver_tx failed: {:?} - {:?}",
                response.code,
                response.info
            );
        }

        Ok(response)
    }

    /// Signals the end of a block.
    async fn end_block(&self, request: request::EndBlock) -> AbciResult<response::EndBlock> {
        tracing::debug!(height = request.height, "end block");

        // TODO: Return events from epoch transitions.
        let ret = self
            .modify_exec_state(|s| self.interpreter.end(s))
            .await
            .context("end failed")?;

        Ok(to_end_block(ret)?)
    }

    /// Commit the current state at the current height.
    async fn commit(&self) -> AbciResult<response::Commit> {
        let exec_state = self.take_exec_state().await;

        // Commit the execution state to the datastore.
        let mut state = self.committed_state()?;
        state.block_height = exec_state.block_height().try_into()?;
        state.state_params.timestamp = exec_state.timestamp();

        let (
            state_root,
            FvmUpdatableParams {
                power_scale,
                circ_supply,
            },
            _,
        ) = exec_state.commit().context("failed to commit FVM")?;

        state.state_params.state_root = state_root;
        state.state_params.power_scale = power_scale;
        state.state_params.circ_supply = circ_supply;

        let app_hash = state.app_hash();
        let block_height = state.block_height;

        tracing::debug!(
            block_height,
            state_root = state_root.to_string(),
            app_hash = app_hash.to_string(),
            timestamp = state.state_params.timestamp.0,
            "commit state"
        );

        // TODO: We can defer committing changes the resolution pool to this point.
        // For example if a checkpoint is successfully executed, that's when we want to remove
        // that checkpoint from the pool, and not propose it to other validators again.
        // However, once Tendermint starts delivering the transactions, the commit will surely
        // follow at the end, so we can also remove these checkpoints from memory at the time
        // the transaction is delivered, rather than when the whole thing is committed.
        // It is only important to the persistent database changes as an atomic step in the
        // commit in case the block execution fails somewhere in the middle for uknown reasons.
        // But if that happened, we will have to restart the application again anyway, and
        // repopulate the in-memory checkpoints based on the last committed ledger.
        // So, while the pool is theoretically part of the evolving state and we can pass
        // it in and out, unless we want to defer commit to here (which the interpreters aren't
        // notified about), we could add it to the `ChainMessageInterpreter` as a constructor argument,
        // a sort of "ambient state", and not worry about in in the `App` at all.

        // Commit app state to the datastore.
        self.set_committed_state(state)?;

        // Reset check state.
        let mut guard = self.check_state.lock().await;
        *guard = None;

        let response = response::Commit {
            data: app_hash.into(),
            // We have to retain blocks until we can support Snapshots.
            retain_height: Default::default(),
        };
        Ok(response)
    }

    /// List the snapshots available on this node to be served to remote peers.
    async fn list_snapshots(&self) -> AbciResult<response::ListSnapshots> {
        if let Some(ref client) = self.snapshots {
            let snapshots = atomically(|| client.list_snapshots()).await;
            Ok(to_snapshots(snapshots)?)
        } else {
            Ok(Default::default())
        }
    }

    /// Load a particular snapshot chunk a remote peer is asking for.
    async fn load_snapshot_chunk(
        &self,
        request: request::LoadSnapshotChunk,
    ) -> AbciResult<response::LoadSnapshotChunk> {
        if let Some(ref client) = self.snapshots {
            if let Some(snapshot) =
                atomically(|| client.access_snapshot(request.height.value(), request.format)).await
            {
                match snapshot.load_chunk(request.chunk) {
                    Ok(chunk) => {
                        return Ok(response::LoadSnapshotChunk {
                            chunk: chunk.into(),
                        });
                    }
                    Err(e) => {
                        tracing::warn!("failed to load chunk: {e:#}");
                    }
                }
            }
        }
        Ok(Default::default())
    }

    /// Decide whether to start downloading a snapshot from peers.
    ///
    /// This method is also called when a download is aborted and a new snapshot is offered,
    /// so potentially we have to clean up previous resources and start a new one.
    async fn offer_snapshot(
        &self,
        request: request::OfferSnapshot,
    ) -> AbciResult<response::OfferSnapshot> {
        if let Some(ref client) = self.snapshots {
            match from_snapshot(request).context("failed to parse snapshot") {
                Ok(manifest) => {
                    tracing::info!(?manifest, "received snapshot offer");
                    // We can look at the version but currently there's only one.
                    match atomically_or_err(|| client.offer_snapshot(manifest.clone())).await {
                        Ok(path) => {
                            tracing::info!(
                                download_dir = path.to_string_lossy().to_string(),
                                height = manifest.block_height,
                                size = manifest.size,
                                chunks = manifest.chunks,
                                "downloading snapshot"
                            );
                            return Ok(response::OfferSnapshot::Accept);
                        }
                        Err(SnapshotError::IncompatibleVersion(version)) => {
                            tracing::warn!(version, "rejecting offered snapshot version");
                            return Ok(response::OfferSnapshot::RejectFormat);
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "failed to start snapshot download");
                            return Ok(response::OfferSnapshot::Abort);
                        }
                    };
                }
                Err(e) => {
                    tracing::warn!("failed to parse snapshot offer: {e:#}");
                    return Ok(response::OfferSnapshot::Reject);
                }
            }
        }
        Ok(Default::default())
    }

    /// Apply the given snapshot chunk to the application's state.
    async fn apply_snapshot_chunk(
        &self,
        request: request::ApplySnapshotChunk,
    ) -> AbciResult<response::ApplySnapshotChunk> {
        tracing::debug!(chunk = request.index, "received snapshot chunk");
        let default = response::ApplySnapshotChunk::default();

        if let Some(ref client) = self.snapshots {
            match atomically_or_err(|| {
                client.save_chunk(request.index, request.chunk.clone().into())
            })
            .await
            {
                Ok(snapshot) => {
                    if let Some(snapshot) = snapshot {
                        tracing::info!(
                            download_dir = snapshot.snapshot_dir.to_string_lossy().to_string(),
                            height = snapshot.manifest.block_height,
                            "received all snapshot chunks",
                        );

                        // Ideally we would import into some isolated store then validate,
                        // but for now let's trust that all is well.
                        if let Err(e) = snapshot.import(self.state_store_clone(), true).await {
                            tracing::error!(error =? e, "failed to import snapshot");
                            return Ok(response::ApplySnapshotChunk {
                                result: response::ApplySnapshotChunkResult::RejectSnapshot,
                                ..default
                            });
                        }

                        tracing::info!(
                            height = snapshot.manifest.block_height,
                            "imported snapshot"
                        );

                        // Now insert the new state into the history.
                        let mut state = self.committed_state()?;
                        state.block_height = snapshot.manifest.block_height;
                        state.state_params = snapshot.manifest.state_params;
                        self.set_committed_state(state)?;

                        // TODO: We can remove the `current_download` from the STM
                        // state here which would cause it to get dropped from /tmp,
                        // but for now let's keep it just in case we need to investigate
                        // some problem.

                        // We could also move the files into our own snapshot directory
                        // so that we can offer it to others, but again let's hold on
                        // until we have done more robust validation.
                    }
                    return Ok(response::ApplySnapshotChunk {
                        result: response::ApplySnapshotChunkResult::Accept,
                        ..default
                    });
                }
                Err(SnapshotError::UnexpectedChunk(expected, got)) => {
                    tracing::warn!(got, expected, "unexpected snapshot chunk index");
                    return Ok(response::ApplySnapshotChunk {
                        result: response::ApplySnapshotChunkResult::Retry,
                        refetch_chunks: vec![expected],
                        ..default
                    });
                }
                Err(SnapshotError::WrongChecksum(expected, got)) => {
                    tracing::warn!(?got, ?expected, "wrong snapshot checksum");
                    // We could retry this snapshot, or try another one.
                    // If we retry, we have to tell which chunks to refetch.
                    return Ok(response::ApplySnapshotChunk {
                        result: response::ApplySnapshotChunkResult::RejectSnapshot,
                        ..default
                    });
                }
                Err(e) => {
                    tracing::error!(
                        chunk = request.index,
                        sender = request.sender,
                        error = ?e,
                        "failed to process snapshot chunk"
                    );
                }
            }
        }

        Ok(default)
    }
}

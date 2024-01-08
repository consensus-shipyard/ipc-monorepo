// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
//! A constant running process that fetch or listener to parent state

mod pointers;
mod syncer;
mod tendermint;

use crate::proxy::ParentQueryProxy;
use crate::sync::syncer::LotusParentSyncer;
use crate::sync::tendermint::TendermintAwareSyncer;
use crate::{CachedFinalityProvider, Config, IPCParentFinality, ParentFinalityProvider, Toggle};
use anyhow::anyhow;
use async_stm::atomically;
use ethers::utils::hex;
use std::sync::Arc;
use std::time::Duration;

/// Query the parent finality from the block chain state
pub trait ParentFinalityStateQuery {
    /// Get the latest committed finality from the state
    fn get_latest_committed_finality(&self) -> anyhow::Result<Option<IPCParentFinality>>;
}

/// Constantly syncing with parent through polling
struct PollingParentSyncer<T, C, P> {
    config: Config,
    parent_view_provider: Arc<Toggle<CachedFinalityProvider<P>>>,
    parent_client: Arc<P>,
    committed_state_query: Arc<T>,
    tendermint_client: C,
}

/// Queries the starting finality for polling. First checks the committed finality, if none, that
/// means the chain has just started, then query from the parent to get the genesis epoch.
async fn query_starting_finality<T, P>(
    query: &Arc<T>,
    parent_client: &Arc<P>,
) -> anyhow::Result<IPCParentFinality>
where
    T: ParentFinalityStateQuery + Send + Sync + 'static,
    P: ParentQueryProxy + Send + Sync + 'static,
{
    loop {
        let mut finality = match query.get_latest_committed_finality() {
            Ok(Some(finality)) => finality,
            Ok(None) => {
                tracing::debug!("app not ready for query yet");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            Err(e) => {
                tracing::warn!(error = e.to_string(), "cannot get committed finality");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        tracing::info!(finality = finality.to_string(), "latest finality committed");

        // this means there are no previous committed finality yet, we fetch from parent to get
        // the genesis epoch of the current subnet and its corresponding block hash.
        if finality.height == 0 {
            let genesis_epoch = parent_client.get_genesis_epoch().await?;
            tracing::debug!(genesis_epoch = genesis_epoch, "obtained genesis epoch");
            let r = parent_client.get_block_hash(genesis_epoch).await?;
            tracing::debug!(
                block_hash = hex::encode(&r.block_hash),
                "obtained genesis block hash",
            );

            finality = IPCParentFinality {
                height: genesis_epoch,
                block_hash: r.block_hash,
            };
            tracing::info!(
                genesis_finality = finality.to_string(),
                "no previous finality committed, fetched from genesis epoch"
            );
        }

        return Ok(finality);
    }
}

/// Start the polling parent syncer in the background
pub async fn launch_polling_syncer<T, C, P>(
    query: T,
    config: Config,
    view_provider: Arc<Toggle<CachedFinalityProvider<P>>>,
    parent_client: Arc<P>,
    tendermint_client: C,
) -> anyhow::Result<()>
where
    T: ParentFinalityStateQuery + Send + Sync + 'static,
    C: tendermint_rpc::Client + Send + Sync + 'static,
    P: ParentQueryProxy + Send + Sync + 'static,
{
    if !view_provider.is_enabled() {
        return Err(anyhow!("provider not enabled, enable to run syncer"));
    }

    let query = Arc::new(query);
    let finality = query_starting_finality(&query, &parent_client).await?;
    atomically(|| view_provider.set_new_finality(finality.clone(), None)).await;

    tracing::info!(
        finality = finality.to_string(),
        "launching parent syncer with last committed finality"
    );

    let poll = PollingParentSyncer::new(
        config,
        view_provider,
        parent_client,
        query,
        tendermint_client,
    );
    poll.start();

    Ok(())
}

impl<T, C, P> PollingParentSyncer<T, C, P> {
    pub fn new(
        config: Config,
        parent_view_provider: Arc<Toggle<CachedFinalityProvider<P>>>,
        parent_client: Arc<P>,
        query: Arc<T>,
        tendermint_client: C,
    ) -> Self {
        Self {
            config,
            parent_view_provider,
            parent_client,
            committed_state_query: query,
            tendermint_client,
        }
    }
}

impl<T, C, P> PollingParentSyncer<T, C, P>
where
    T: ParentFinalityStateQuery + Send + Sync + 'static,
    C: tendermint_rpc::Client + Send + Sync + 'static,
    P: ParentQueryProxy + Send + Sync + 'static,
{
    /// Start the parent finality listener in the background
    pub fn start(self) {
        let config = self.config;
        let provider = self.parent_view_provider;
        let parent_client = self.parent_client;
        let query = self.committed_state_query;
        let tendermint_client = self.tendermint_client;

        let mut interval = tokio::time::interval(config.polling_interval);

        tokio::spawn(async move {
            let lotus_syncer = LotusParentSyncer::new(config, parent_client, provider, query)
                .await
                .expect("");
            let mut tendermint_syncer = TendermintAwareSyncer::new(lotus_syncer, tendermint_client);

            loop {
                interval.tick().await;

                if let Err(e) = tendermint_syncer.sync().await {
                    tracing::error!(error = e.to_string(), "sync with parent encountered error");
                }
            }
        });
    }
}

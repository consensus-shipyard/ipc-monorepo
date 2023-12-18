// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
//! Bottom up checkpoint manager

use crate::config::Subnet;
use crate::manager::{BottomUpCheckpointRelayer, EthSubnetManager};
use anyhow::{anyhow, Result};
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use ipc_identity::{EthKeyAddress, PersistentKeyStore};
use std::cmp::max;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Tracks the config required for bottom up checkpoint submissions
/// parent/child subnet and checkpoint period.
pub struct CheckpointConfig {
    parent: Subnet,
    child: Subnet,
    period: ChainEpoch,
}

/// Manages the submission of bottom up checkpoint. It checks if the submitter has already
/// submitted in the `last_checkpoint_height`, if not, it will submit the checkpoint at that height.
/// Then it will submit at the next submission height for the new checkpoint.
pub struct BottomUpCheckpointManager<T> {
    metadata: CheckpointConfig,
    parent_handler: T,
    child_handler: T,
    /// The number of blocks away from the chain head that is considered final
    finalization_blocks: ChainEpoch,
}

impl<T: BottomUpCheckpointRelayer> BottomUpCheckpointManager<T> {
    pub async fn new(
        parent: Subnet,
        child: Subnet,
        parent_handler: T,
        child_handler: T,
    ) -> Result<Self> {
        let period = parent_handler
            .checkpoint_period(&child.id)
            .await
            .map_err(|e| anyhow!("cannot get bottom up checkpoint period: {e}"))?;
        Ok(Self {
            metadata: CheckpointConfig {
                parent,
                child,
                period,
            },
            parent_handler,
            child_handler,
            finalization_blocks: 0,
        })
    }

    pub fn with_finalization_blocks(mut self, finalization_blocks: ChainEpoch) -> Self {
        self.finalization_blocks = finalization_blocks;
        self
    }
}

impl BottomUpCheckpointManager<EthSubnetManager> {
    pub async fn new_evm_manager(
        parent: Subnet,
        child: Subnet,
        keystore: Arc<RwLock<PersistentKeyStore<EthKeyAddress>>>,
    ) -> Result<Self> {
        let parent_handler =
            EthSubnetManager::from_subnet_with_wallet_store(&parent, Some(keystore.clone()))?;
        let child_handler =
            EthSubnetManager::from_subnet_with_wallet_store(&child, Some(keystore))?;
        Self::new(parent, child, parent_handler, child_handler).await
    }
}

impl<T: BottomUpCheckpointRelayer> Display for BottomUpCheckpointManager<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bottom-up relayer, parent: {:}, child: {:}",
            self.metadata.parent.id, self.metadata.child.id
        )
    }
}

impl<T: BottomUpCheckpointRelayer + Send + Sync + 'static> BottomUpCheckpointManager<T> {
    /// Getter for the parent subnet this checkpoint manager is handling
    pub fn parent_subnet(&self) -> &Subnet {
        &self.metadata.parent
    }

    /// Getter for the target subnet this checkpoint manager is handling
    pub fn child_subnet(&self) -> &Subnet {
        &self.metadata.child
    }

    /// The checkpoint period that the current manager is submitting upon
    pub fn checkpoint_period(&self) -> ChainEpoch {
        self.metadata.period
    }

    /// Run the bottom up checkpoint submission daemon in the foreground
    pub async fn run(self, submitter: Address, submission_interval: Duration) {
        log::info!("launching {self} for {submitter}");

        loop {
            if let Err(e) = self.submit_checkpoint(&submitter).await {
                log::error!("cannot submit checkpoint for submitter: {submitter} due to {e}");
            }

            tokio::time::sleep(submission_interval).await;
        }
    }

    /// Submit the checkpoint from the target submitter address
    pub async fn submit_checkpoint(&self, submitter: &Address) -> Result<()> {
        self.submit_last_epoch(submitter).await?;
        self.submit_next_epoch(submitter).await
    }

    /// Derive the next submission checkpoint height
    async fn next_submission_height(&self) -> Result<ChainEpoch> {
        let last_checkpoint_epoch = self
            .parent_handler
            .last_bottom_up_checkpoint_height(&self.metadata.child.id)
            .await
            .map_err(|e| {
                anyhow!("cannot obtain the last bottom up checkpoint height due to: {e:}")
            })?;
        Ok(last_checkpoint_epoch + self.checkpoint_period())
    }

    /// Checks if the relayer has already submitted at the `last_checkpoint_height`, if not it submits it.
    async fn submit_last_epoch(&self, submitter: &Address) -> Result<()> {
        let subnet = &self.metadata.child.id;
        if self
            .parent_handler
            .has_submitted_in_last_checkpoint_height(subnet, submitter)
            .await?
        {
            return Ok(());
        }

        let height = self
            .parent_handler
            .last_bottom_up_checkpoint_height(subnet)
            .await?;

        if height == 0 {
            log::debug!("no previous checkpoint yet");
            return Ok(());
        }

        let bundle = self.child_handler.checkpoint_bundle_at(height).await?;
        log::debug!("bottom up bundle: {bundle:?}");

        let epoch = self
            .parent_handler
            .submit_checkpoint(submitter, bundle)
            .await
            .map_err(|e| anyhow!("cannot submit bottom up checkpoint due to: {e:}"))?;
        log::info!(
            "submitted bottom up checkpoint({}) in parent at height {}",
            height,
            epoch
        );

        Ok(())
    }

    /// Checks if the relayer has already submitted at the next submission epoch, if not it submits it.
    async fn submit_next_epoch(&self, submitter: &Address) -> Result<()> {
        let next_submission_height = self.next_submission_height().await?;
        let current_height = self.child_handler.current_epoch().await?;
        let finalized_height = max(1, current_height - self.finalization_blocks);

        log::debug!("next_submission_height: {next_submission_height}, current height: {current_height}, finalized_height: {finalized_height}");

        if finalized_height < next_submission_height {
            return Ok(());
        }

        let prev_h = next_submission_height - self.checkpoint_period();
        log::debug!("start querying quorum reached events from : {prev_h} to {finalized_height}");

        for h in (prev_h + 1)..=finalized_height {
            let events = self.child_handler.quorum_reached_events(h).await?;
            if events.is_empty() {
                log::debug!("no reached events at height : {h}");
                continue;
            }

            log::debug!("found reached events at height : {h}");

            for event in events {
                let bundle = self
                    .child_handler
                    .checkpoint_bundle_at(event.height)
                    .await?;
                log::debug!("bottom up bundle: {bundle:?}");

                let epoch = self
                    .parent_handler
                    .submit_checkpoint(submitter, bundle)
                    .await
                    .map_err(|e| anyhow!("cannot submit bottom up checkpoint due to: {e:}"))?;

                log::info!(
                    "submitted bottom up checkpoint({}) in parent at height {}",
                    event.height,
                    epoch
                );
            }
        }

        Ok(())
    }
}

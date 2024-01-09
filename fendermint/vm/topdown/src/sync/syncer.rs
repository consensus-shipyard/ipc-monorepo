// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
//! The inner type of parent syncer

use crate::finality::ParentViewPayload;
use crate::proxy::ParentQueryProxy;
use crate::sync::pointers::SyncPointers;
use crate::sync::{query_starting_finality, ParentFinalityStateQuery};
use crate::{
    is_null_round_str, BlockHash, BlockHeight, CachedFinalityProvider, Config, Error, Toggle,
};
use anyhow::anyhow;
use async_stm::{atomically, atomically_or_err};
use ethers::utils::hex;
use std::sync::Arc;

/// Parent syncer that constantly poll parent. This struct handles lotus null blocks and deferred
/// execution. For ETH based parent, it should work out of the box as well.
pub(crate) struct LotusParentSyncer<T, P> {
    config: Config,
    parent_proxy: Arc<P>,
    provider: Arc<Toggle<CachedFinalityProvider<P>>>,
    query: Arc<T>,

    /// The pointers that indicate which height to poll parent next
    sync_pointers: SyncPointers,
}

impl<T, P> LotusParentSyncer<T, P>
where
    T: ParentFinalityStateQuery + Send + Sync + 'static,
    P: ParentQueryProxy + Send + Sync + 'static,
{
    pub async fn new(
        config: Config,
        parent_proxy: Arc<P>,
        provider: Arc<Toggle<CachedFinalityProvider<P>>>,
        query: Arc<T>,
    ) -> anyhow::Result<Self> {
        let last_committed_finality = atomically(|| provider.last_committed_finality())
            .await
            .ok_or_else(|| anyhow!("parent finality not ready"))?;

        Ok(Self {
            config,
            parent_proxy,
            provider,
            query,
            sync_pointers: SyncPointers::new(last_committed_finality.height),
        })
    }

    /// There are 2 pointers, each refers to a block height, when syncing with parent. As Lotus has
    /// delayed execution and null round, we need to ensure the topdown messages and validator
    /// changes polled are indeed finalized and executed. The following three pointers are introduced:
    ///     - tail: The next block height in cache to be confirmed executed, could be None
    ///     - head: The latest block height fetched in cache, finalized but may not be executed.
    ///
    /// Say we have block chain as follows:
    /// NonNullBlock(1) -> NonNullBlock(2) -> NullBlock(3) -> NonNullBlock(4) -> NullBlock(5) -> NonNullBlock(6)
    /// and block height 1 is the previously finalized and executed block height.
    ///
    /// At the beginning, head == 1 and tail == None. With a new block height fetched,
    /// `head = 2`. Since height at 2 is not a null block, `tail = Some(2)`, because we cannot be sure
    /// block 2 has executed yet. When a new block is fetched, `head = 3`. Since head is a null block, we
    /// cannot confirm block height 2. When `head = 4`, it's not a null block, we can confirm block 2 is
    /// executed (also with some checks to ensure no reorg has occurred). We fetch block 2's data and set
    /// `tail = Some(4)`.
    /// The data fetch at block height 2 is pushed to cache and height 2 is ready to be proposed.
    ///
    /// At height 6, it's block height 4 will be confirmed and its data pushed to cache. At the same
    /// time, since block 3 is a null block, empty data will also be pushed to cache. Block 4 is ready
    /// to be proposed.
    pub async fn sync(&mut self) -> anyhow::Result<()> {
        let chain_head = if let Some(h) = self.finalized_chain_head().await? {
            h
        } else {
            return Ok(());
        };
        tracing::debug!(
            chain_head,
            pointers = self.sync_pointers.to_string(),
            "syncing heights"
        );

        if self.detected_reorg_by_height(chain_head) {
            tracing::warn!(
                pointers = self.sync_pointers.to_string(),
                chain_head,
                "reorg detected from height"
            );
            return self.reset_cache().await;
        }

        if !self.has_new_blocks(chain_head) {
            tracing::debug!("the parent has yet to produce a new block");
            return Ok(());
        }

        if self.exceed_cache_size_limit().await {
            tracing::debug!("exceeded cache size limit");
            return Ok(());
        }

        self.poll_next().await?;

        Ok(())
    }
}

impl<T, P> LotusParentSyncer<T, P>
where
    T: ParentFinalityStateQuery + Send + Sync + 'static,
    P: ParentQueryProxy + Send + Sync + 'static,
{
    async fn exceed_cache_size_limit(&self) -> bool {
        let max_cache_blocks = self.config.max_cache_blocks();
        atomically(|| self.provider.cached_blocks()).await > max_cache_blocks
    }

    /// Poll the next block height. Returns finalized and executed block data.
    async fn poll_next(&mut self) -> Result<(), Error> {
        let height = self.sync_pointers.head() + 1;
        let parent_block_hash = self.non_null_parent_hash().await;

        tracing::debug!(
            height,
            parent_block_hash = hex::encode(&parent_block_hash),
            "polling height with parent hash"
        );

        let block_hash_res = match self.parent_proxy.get_block_hash(height).await {
            Ok(res) => res,
            Err(e) => {
                let err = e.to_string();
                if is_null_round_str(&err) {
                    tracing::debug!(height, "detected null round at height");

                    self.sync_pointers.advance_head();

                    return Ok(());
                }
                return Err(Error::CannotQueryParent(err, height));
            }
        };

        if block_hash_res.parent_block_hash != parent_block_hash {
            tracing::warn!(
                height,
                parent_hash = hex::encode(&block_hash_res.parent_block_hash),
                previous_hash = hex::encode(&parent_block_hash),
                "parent block hash diff than previous hash",
            );
            return Err(Error::ParentChainReorgDetected);
        }

        if let Some((to_confirm_height, to_confirm_hash)) = self.sync_pointers.tail() {
            tracing::debug!(
                height,
                confirm = to_confirm_height,
                "non-null round at height, confirmed previous height"
            );

            let data = self.fetch_data(to_confirm_height, to_confirm_hash).await?;
            atomically_or_err::<_, Error, _>(|| {
                // we only push the null block in cache when we confirmed a block so that in cache
                // the latest height is always a confirmed non null block.
                let latest_height = self
                    .provider
                    .latest_height()?
                    .expect("provider contains data at this point");
                for h in (latest_height + 1)..to_confirm_height {
                    self.provider.new_parent_view(h, None)?;
                    tracing::debug!(height = h, "found null block pushed to cache");
                }
                self.provider
                    .new_parent_view(to_confirm_height, Some(data.clone()))?;
                tracing::debug!(height = to_confirm_height, "non-null block pushed to cache");
                Ok(())
            })
            .await?;
        } else {
            tracing::debug!(height, "non-null round at height, waiting for confirmation");
        };

        self.sync_pointers
            .set_tail(height, block_hash_res.block_hash);
        self.sync_pointers.advance_head();

        Ok(())
    }

    async fn fetch_data(
        &self,
        height: BlockHeight,
        block_hash: BlockHash,
    ) -> Result<ParentViewPayload, Error> {
        let changes_res = self
            .parent_proxy
            .get_validator_changes(height)
            .await
            .map_err(|e| Error::CannotQueryParent(e.to_string(), height))?;
        if changes_res.block_hash != block_hash {
            tracing::warn!(
                height,
                change_set_hash = hex::encode(&changes_res.block_hash),
                block_hash = hex::encode(&block_hash),
                "change set block hash does not equal block hash",
            );
            return Err(Error::ParentChainReorgDetected);
        }

        let topdown_msgs_res = self
            .parent_proxy
            .get_top_down_msgs(height)
            .await
            .map_err(|e| Error::CannotQueryParent(e.to_string(), height))?;
        if topdown_msgs_res.block_hash != block_hash {
            tracing::warn!(
                height,
                topdown_msgs_hash = hex::encode(&topdown_msgs_res.block_hash),
                block_hash = hex::encode(&block_hash),
                "topdown messages block hash does not equal block hash",
            );
            return Err(Error::ParentChainReorgDetected);
        }

        Ok((block_hash, changes_res.value, topdown_msgs_res.value))
    }

    /// We only want the non-null parent block's hash
    async fn non_null_parent_hash(&self) -> BlockHash {
        if let Some((height, hash)) = self.sync_pointers.tail() {
            tracing::debug!(
                pending_height = height,
                "previous non null parent is the pending confirmation block"
            );
            return hash;
        };

        atomically(|| {
            Ok(if let Some(h) = self.provider.latest_height_in_cache()? {
                tracing::debug!(
                    previous_confirmed_height = h,
                    "found previous non null block in cache"
                );
                // safe to unwrap as we have height recorded
                self.provider.block_hash(h)?.unwrap()
            } else if let Some(p) = self.provider.last_committed_finality()? {
                tracing::debug!(
                    previous_confirmed_height = p.height,
                    "no cache, found previous non null block as last committed finality"
                );
                p.block_hash
            } else {
                unreachable!("guaranteed to non null block hash, report bug please")
            })
        })
        .await
    }

    fn has_new_blocks(&self, height: BlockHeight) -> bool {
        self.sync_pointers.head() < height
    }

    fn detected_reorg_by_height(&self, height: BlockHeight) -> bool {
        // If the below is true, we are going backwards in terms of block height, the latest block
        // height is lower than our previously fetched head. It could be a chain reorg.
        self.sync_pointers.head() > height
    }

    async fn finalized_chain_head(&self) -> anyhow::Result<Option<BlockHeight>> {
        let parent_chain_head_height = self.parent_proxy.get_chain_head_height().await?;
        // sanity check
        if parent_chain_head_height < self.config.chain_head_delay {
            tracing::debug!("latest height not more than the chain head delay");
            return Ok(None);
        }

        // we consider the chain head finalized only after the `chain_head_delay`
        Ok(Some(
            parent_chain_head_height - self.config.chain_head_delay,
        ))
    }

    /// Reset the cache in the face of a reorg
    async fn reset_cache(&self) -> anyhow::Result<()> {
        let finality = query_starting_finality(&self.query, &self.parent_proxy).await?;
        atomically(|| self.provider.reset(finality.clone())).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::proxy::ParentQueryProxy;
    use crate::sync::syncer::LotusParentSyncer;
    use crate::sync::ParentFinalityStateQuery;
    use crate::{
        BlockHash, BlockHeight, CachedFinalityProvider, Config, IPCParentFinality,
        SequentialKeyCache, Toggle, NULL_ROUND_ERR_MSG,
    };
    use anyhow::anyhow;
    use async_stm::atomically;
    use async_trait::async_trait;
    use ipc_api::cross::CrossMsg;
    use ipc_api::staking::StakingChangeRequest;
    use ipc_provider::manager::{GetBlockHashResult, TopDownQueryPayload};
    use std::sync::Arc;

    struct TestParentFinalityStateQuery {
        latest_finality: IPCParentFinality,
    }

    impl ParentFinalityStateQuery for TestParentFinalityStateQuery {
        fn get_latest_committed_finality(&self) -> anyhow::Result<Option<IPCParentFinality>> {
            Ok(Some(self.latest_finality.clone()))
        }
    }

    struct TestParentProxy {
        blocks: SequentialKeyCache<BlockHeight, Option<BlockHash>>,
    }

    #[async_trait]
    impl ParentQueryProxy for TestParentProxy {
        async fn get_chain_head_height(&self) -> anyhow::Result<BlockHeight> {
            Ok(self.blocks.upper_bound().unwrap())
        }

        async fn get_genesis_epoch(&self) -> anyhow::Result<BlockHeight> {
            Ok(self.blocks.lower_bound().unwrap() - 1)
        }

        async fn get_block_hash(&self, height: BlockHeight) -> anyhow::Result<GetBlockHashResult> {
            let r = self.blocks.get_value(height).unwrap();
            if r.is_none() {
                return Err(anyhow!(NULL_ROUND_ERR_MSG));
            }

            for h in (self.blocks.lower_bound().unwrap()..height).rev() {
                let v = self.blocks.get_value(h).unwrap();
                if v.is_none() {
                    continue;
                }
                return Ok(GetBlockHashResult {
                    parent_block_hash: v.clone().unwrap(),
                    block_hash: r.clone().unwrap(),
                });
            }
            panic!("invalid testing data")
        }

        async fn get_top_down_msgs(
            &self,
            height: BlockHeight,
        ) -> anyhow::Result<TopDownQueryPayload<Vec<CrossMsg>>> {
            Ok(TopDownQueryPayload {
                value: vec![],
                block_hash: self.blocks.get_value(height).cloned().unwrap().unwrap(),
            })
        }

        async fn get_validator_changes(
            &self,
            height: BlockHeight,
        ) -> anyhow::Result<TopDownQueryPayload<Vec<StakingChangeRequest>>> {
            Ok(TopDownQueryPayload {
                value: vec![],
                block_hash: self.blocks.get_value(height).cloned().unwrap().unwrap(),
            })
        }
    }

    async fn new_syncer(
        blocks: SequentialKeyCache<BlockHeight, Option<BlockHash>>,
    ) -> LotusParentSyncer<TestParentFinalityStateQuery, TestParentProxy> {
        let config = Config {
            chain_head_delay: 2,
            polling_interval: Default::default(),
            exponential_back_off: Default::default(),
            exponential_retry_limit: 0,
            max_proposal_range: Some(1),
            max_cache_blocks: None,
            proposal_delay: None,
        };
        let genesis_epoch = blocks.lower_bound().unwrap();
        let proxy = Arc::new(TestParentProxy { blocks });
        let committed_finality = IPCParentFinality {
            height: genesis_epoch,
            block_hash: vec![0; 32],
        };

        let provider = CachedFinalityProvider::new(
            config.clone(),
            genesis_epoch,
            Some(committed_finality.clone()),
            proxy.clone(),
        );
        LotusParentSyncer::new(
            config,
            proxy,
            Arc::new(Toggle::enabled(provider)),
            Arc::new(TestParentFinalityStateQuery {
                latest_finality: committed_finality,
            }),
        )
        .await
        .unwrap()
    }

    /// Creates a mock of a new parent blockchain view. The key is the height and the value is the
    /// block hash. If block hash is None, it means the current height is a null block.
    macro_rules! new_parent_blocks {
        ($($key:expr => $val:expr),* ,) => (
            hash_map!($($key => $val),*)
        );
        ($($key:expr => $val:expr),*) => ({
            let mut map = SequentialKeyCache::sequential();
            $( map.append($key, $val).unwrap(); )*
            map
        });
    }

    #[tokio::test]
    async fn happy_path() {
        let parent_blocks = new_parent_blocks!(
            100 => Some(vec![0; 32]),   // genesis block
            101 => Some(vec![1; 32]),
            102 => Some(vec![2; 32]),
            103 => Some(vec![3; 32]),
            104 => Some(vec![4; 32]),
            105 => Some(vec![5; 32])    // chain head
        );

        let mut syncer = new_syncer(parent_blocks).await;

        assert_eq!(syncer.sync_pointers.head(), 100);
        assert_eq!(syncer.sync_pointers.tail(), None);

        // sync block 101, which is a non-null block
        let r = syncer.sync().await;
        assert!(r.is_ok());
        assert_eq!(syncer.sync_pointers.head(), 101);
        assert_eq!(syncer.sync_pointers.tail(), Some((101, vec![1; 32])));
        // latest height is None as we are yet to confirm block 101, so latest height should equal
        // to the last committed finality initialized, which is the genesis block 100
        assert_eq!(
            atomically(|| syncer.provider.latest_height()).await,
            Some(100)
        );

        // sync block 101, which is a non-null block
        let r = syncer.sync().await;
        assert!(r.is_ok());
        assert_eq!(syncer.sync_pointers.head(), 102);
        assert_eq!(syncer.sync_pointers.tail(), Some((102, vec![2; 32])));
        assert_eq!(
            atomically(|| syncer.provider.latest_height()).await,
            Some(101)
        );
    }

    #[tokio::test]
    async fn with_non_null_block() {
        let parent_blocks = new_parent_blocks!(
            100 => Some(vec![0; 32]),   // genesis block
            101 => None,
            102 => None,
            103 => None,
            104 => Some(vec![4; 32]),
            105 => None,
            106 => None,
            107 => None,
            108 => Some(vec![5; 32]),
            109 => None,
            110 => None,
            111 => None
        );

        let mut syncer = new_syncer(parent_blocks).await;

        assert_eq!(syncer.sync_pointers.head(), 100);
        assert_eq!(syncer.sync_pointers.tail(), None);

        // sync block 101 to 103, which are null blocks
        for h in 101..=103 {
            let r = syncer.sync().await;
            assert!(r.is_ok());
            assert_eq!(syncer.sync_pointers.head(), h);
            assert_eq!(syncer.sync_pointers.tail(), None);
        }

        // sync block 104, which is a non-null block
        syncer.sync().await.unwrap();
        assert_eq!(syncer.sync_pointers.head(), 104);
        assert_eq!(syncer.sync_pointers.tail(), Some((104, vec![4; 32])));
        // latest height is None as we are yet to confirm block 104, so latest height should equal
        // to the last committed finality initialized, which is the genesis block 100
        assert_eq!(
            atomically(|| syncer.provider.latest_height()).await,
            Some(100)
        );

        // sync block 105 to 107, which are null blocks
        for h in 105..=107 {
            let r = syncer.sync().await;
            assert!(r.is_ok());
            assert_eq!(syncer.sync_pointers.head(), h);
            assert_eq!(syncer.sync_pointers.tail(), Some((104, vec![4; 32])));
        }

        // sync block 108, which is a non-null block
        syncer.sync().await.unwrap();
        assert_eq!(syncer.sync_pointers.head(), 108);
        assert_eq!(syncer.sync_pointers.tail(), Some((108, vec![5; 32])));
        // latest height is None as we are yet to confirm block 108, so latest height should equal
        // to the previous confirmed block, which is 104
        assert_eq!(
            atomically(|| syncer.provider.latest_height()).await,
            Some(104)
        );
    }
}

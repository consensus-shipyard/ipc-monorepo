// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::{anyhow, Context};
use ethers::types as et;

use fendermint_vm_message::conv::{from_eth, from_fvm};
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::ActorID;

use fendermint_crypto::SecretKey;
use fendermint_vm_actor_interface::{
    eam::EthAddress,
    init::builtin_actor_eth_addr,
    ipc::{AbiHash, ValidatorMerkleTree, GATEWAY_ACTOR_ID},
};
use fendermint_vm_genesis::{Power, Validator};
use fendermint_vm_message::signed::sign_secp256k1;
use fendermint_vm_topdown::IPCParentFinality;
use ipc_actors_abis::gateway_getter_facet::GatewayGetterFacet;
use ipc_actors_abis::gateway_getter_facet::{self as getter, gateway_getter_facet};
use ipc_actors_abis::gateway_router_facet as router;
use ipc_actors_abis::gateway_router_facet::GatewayRouterFacet;
use ipc_sdk::cross::CrossMsg;
use ipc_sdk::staking::StakingChangeRequest;

use super::{
    fevm::{ContractCaller, MockProvider, NoRevert},
    FvmExecState,
};
use crate::fvm::FvmApplyRet;
use fendermint_vm_actor_interface::ipc;
use fvm_shared::econ::TokenAmount;

#[derive(Clone)]
pub struct GatewayCaller<DB> {
    addr: EthAddress,
    getter: ContractCaller<DB, GatewayGetterFacet<MockProvider>, NoRevert>,
    router: ContractCaller<DB, GatewayRouterFacet<MockProvider>, router::GatewayRouterFacetErrors>,
}

impl<DB> Default for GatewayCaller<DB> {
    fn default() -> Self {
        Self::new(GATEWAY_ACTOR_ID)
    }
}

impl<DB> GatewayCaller<DB> {
    pub fn new(actor_id: ActorID) -> Self {
        // A masked ID works for invoking the contract, but internally the EVM uses a different
        // ID and if we used this address for anything like validating that the sender is the gateway,
        // we'll face bitter disappointment. For that we have to use the delegated address we have in genesis.
        let addr = builtin_actor_eth_addr(actor_id);
        Self {
            addr,
            getter: ContractCaller::new(addr, GatewayGetterFacet::new),
            router: ContractCaller::new(addr, GatewayRouterFacet::new),
        }
    }

    pub fn addr(&self) -> EthAddress {
        self.addr
    }
}

impl<DB: Blockstore> GatewayCaller<DB> {
    /// Check that IPC is configured in this deployment.
    pub fn enabled(&self, state: &mut FvmExecState<DB>) -> anyhow::Result<bool> {
        match state.state_tree_mut().get_actor(GATEWAY_ACTOR_ID)? {
            None => Ok(false),
            Some(a) => Ok(!state.builtin_actors().is_placeholder_actor(&a.code)),
        }
    }

    /// Return true if the current subnet is the root subnet.
    pub fn is_root(&self, state: &mut FvmExecState<DB>) -> anyhow::Result<bool> {
        self.subnet_id(state).map(|id| id.route.is_empty())
    }

    /// Return the current subnet ID.
    pub fn subnet_id(&self, state: &mut FvmExecState<DB>) -> anyhow::Result<getter::SubnetID> {
        self.getter.call(state, |c| c.get_network_name())
    }

    /// Fetch the period with which the current subnet has to submit checkpoints to its parent.
    pub fn bottom_up_check_period(&self, state: &mut FvmExecState<DB>) -> anyhow::Result<u64> {
        self.getter.call(state, |c| c.bottom_up_check_period())
    }

    /// Fetch the bottom-up messages enqueued for a given checkpoint height.
    pub fn bottom_up_msgs(
        &self,
        state: &mut FvmExecState<DB>,
        height: u64,
    ) -> anyhow::Result<Vec<getter::CrossMsg>> {
        self.getter.call(state, |c| c.bottom_up_messages(height))
    }

    /// Insert a new checkpoint at the period boundary.
    pub fn create_bottom_up_checkpoint(
        &self,
        state: &mut FvmExecState<DB>,
        checkpoint: router::BottomUpCheckpoint,
        power_table: &[Validator<Power>],
    ) -> anyhow::Result<()> {
        // Construct a Merkle tree from the power table, which we can use to validate validator set membership
        // when the signatures are submitted in transactions for accumulation.
        let tree =
            ValidatorMerkleTree::new(power_table).context("failed to create validator tree")?;

        let total_power = power_table.iter().fold(et::U256::zero(), |p, v| {
            p.saturating_add(et::U256::from(v.power.0))
        });

        self.router.call(state, |c| {
            c.create_bottom_up_checkpoint(checkpoint, tree.root_hash().0, total_power)
        })
    }

    /// Retrieve checkpoints which have not reached a quorum.
    pub fn incomplete_checkpoints(
        &self,
        state: &mut FvmExecState<DB>,
    ) -> anyhow::Result<Vec<getter::BottomUpCheckpoint>> {
        self.getter.call(state, |c| c.get_incomplete_checkpoints())
    }

    /// Apply all pending validator changes, returning the newly adopted configuration number, or 0 if there were no changes.
    pub fn apply_validator_changes(&self, state: &mut FvmExecState<DB>) -> anyhow::Result<u64> {
        self.router.call(state, |c| c.apply_finality_changes())
    }

    /// Get the currently active validator set.
    pub fn current_validator_set(
        &self,
        state: &mut FvmExecState<DB>,
    ) -> anyhow::Result<getter::Membership> {
        self.getter.call(state, |c| c.get_current_membership())
    }

    /// Construct the input parameters for adding a signature to the checkpoint.
    ///
    /// This will need to be broadcasted as a transaction.
    pub fn add_checkpoint_signature_calldata(
        &self,
        checkpoint: router::BottomUpCheckpoint,
        power_table: &[Validator<Power>],
        validator: &Validator<Power>,
        secret_key: &SecretKey,
    ) -> anyhow::Result<et::Bytes> {
        debug_assert_eq!(validator.public_key.0, secret_key.public_key());

        let height = checkpoint.block_height;
        let weight = et::U256::from(validator.power.0);

        let hash = checkpoint.abi_hash();

        let signature = sign_secp256k1(secret_key, &hash);
        let signature =
            from_fvm::to_eth_signature(&signature, false).context("invalid signature")?;
        let signature = et::Bytes::from(signature.to_vec());

        let tree =
            ValidatorMerkleTree::new(power_table).context("failed to construct Merkle tree")?;

        let membership_proof = tree
            .prove(validator)
            .context("failed to construct Merkle proof")?
            .into_iter()
            .map(|p| p.into())
            .collect();

        let call = self.router.contract().add_checkpoint_signature(
            height,
            membership_proof,
            weight,
            signature,
        );

        let calldata = call
            .calldata()
            .ok_or_else(|| anyhow!("no calldata for adding signature"))?;

        Ok(calldata)
    }

    /// Commit the parent finality to the gateway and returns the previously committed finality.
    /// None implies there is no previously committed finality.
    pub fn commit_parent_finality(
        &self,
        state: &mut FvmExecState<DB>,
        finality: IPCParentFinality,
    ) -> anyhow::Result<Option<IPCParentFinality>> {
        let evm_finality = router::ParentFinality::try_from(finality)?;

        let (has_committed, prev_finality) = self
            .router
            .call(state, |c| c.commit_parent_finality(evm_finality))?;

        Ok(if !has_committed {
            None
        } else {
            Some(IPCParentFinality::try_from(prev_finality)?)
        })
    }

    pub fn store_validator_changes(
        &self,
        state: &mut FvmExecState<DB>,
        changes: Vec<StakingChangeRequest>,
    ) -> anyhow::Result<()> {
        if changes.is_empty() {
            return Ok(());
        }

        let mut change_requests = vec![];
        for c in changes {
            change_requests.push(router::StakingChangeRequest::try_from(c)?);
        }

        self.router
            .call(state, |c| c.store_validator_changes(change_requests))
    }

    /// Call this function to mint some FIL to the gateway contract
    pub fn mint_to_gateway(
        &self,
        state: &mut FvmExecState<DB>,
        value: TokenAmount,
    ) -> anyhow::Result<()> {
        let state_tree = state.state_tree_mut();
        state_tree.mutate_actor(ipc::GATEWAY_ACTOR_ID, |actor_state| {
            actor_state.balance += value;
            Ok(())
        })?;
        Ok(())
    }

    pub fn apply_cross_messages(
        &self,
        state: &mut FvmExecState<DB>,
        cross_messages: Vec<CrossMsg>,
    ) -> anyhow::Result<FvmApplyRet> {
        let messages = cross_messages
            .into_iter()
            .map(router::CrossMsg::try_from)
            .collect::<Result<Vec<_>, _>>()
            .context("failed to convert cross messages")?;
        let r = self
            .router
            .call_with_return(state, |c| c.apply_cross_messages(messages))?;
        Ok(r.into_return())
    }

    pub fn get_latest_parent_finality(
        &self,
        state: &mut FvmExecState<DB>,
    ) -> anyhow::Result<IPCParentFinality> {
        let r = self
            .getter
            .call(state, |c| c.get_latest_parent_finality())?;
        Ok(IPCParentFinality::try_from(r)?)
    }

    /// Get the Ethereum adresses of validators who signed a checkpoint.
    pub fn checkpoint_signatories(
        &self,
        state: &mut FvmExecState<DB>,
        height: u64,
    ) -> anyhow::Result<Vec<EthAddress>> {
        let (_, _, addrs, _) = self
            .getter
            .call(state, |c| c.get_signature_bundle(height))?;

        let addrs = addrs.into_iter().map(|a| a.into()).collect();

        Ok(addrs)
    }
}

/// Total amount of tokens to mint as a result of top-down messages arriving at the subnet.
pub fn tokens_to_mint(msgs: &[ipc_sdk::cross::CrossMsg]) -> TokenAmount {
    msgs.iter()
        .fold(TokenAmount::from_atto(0), |mut total, msg| {
            // Both fees and value are considered to enter the ciruculating supply of the subnet.
            // Fees might be distributed among subnet validators.
            total += &msg.msg.value;
            total += &msg.msg.fee;
            total
        })
}

/// Total amount of tokens to burn as a result of bottom-up messages leaving the subnet.
pub fn tokens_to_burn(msgs: &[gateway_getter_facet::CrossMsg]) -> TokenAmount {
    msgs.iter()
        .fold(TokenAmount::from_atto(0), |mut total, msg| {
            // Both fees and value were taken from the sender, and both are going up to the parent subnet:
            // https://github.com/consensus-shipyard/ipc-solidity-actors/blob/e4ec0046e2e73e2f91d7ab8ae370af2c487ce526/src/gateway/GatewayManagerFacet.sol#L143-L150
            // Fees might be distirbuted among relayers.
            total += from_eth::to_fvm_tokens(&msg.message.value);
            total += from_eth::to_fvm_tokens(&msg.message.fee);
            total
        })
}

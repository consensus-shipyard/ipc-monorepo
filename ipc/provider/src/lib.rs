// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
//! Ipc agent sdk, contains the json rpc client to interact with the IPC agent rpc server.

use crate::manager::{GetBlockHashResult, TopDownQueryPayload};
use anyhow::anyhow;
use base64::Engine;
use config::Config;
use fvm_shared::{
    address::Address, clock::ChainEpoch, crypto::signature::SignatureType, econ::TokenAmount,
};
use ipc_identity::{
    EthKeyAddress, EvmKeyStore, KeyStore, KeyStoreConfig, PersistentKeyStore, Wallet,
};
use ipc_sdk::checkpoint::{BottomUpCheckpointBundle, QuorumReachedEvent};
use ipc_sdk::staking::{StakingChangeRequest, ValidatorInfo};
use ipc_sdk::{
    cross::CrossMsg,
    subnet::{ConsensusType, ConstructParams},
    subnet_id::SubnetID,
};
use lotus::message::wallet::WalletKeyType;
use manager::{EthSubnetManager, SubnetGenesisInfo, SubnetInfo, SubnetManager};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Borrow,
    collections::HashMap,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, RwLock},
};
use zeroize::Zeroize;

pub mod checkpoint;
pub mod config;
pub mod jsonrpc;
pub mod lotus;
pub mod manager;

const DEFAULT_REPO_PATH: &str = ".ipc";
const DEFAULT_CONFIG_NAME: &str = "config.toml";

/// The subnet manager connection that holds the subnet config and the manager instance.
pub struct Connection {
    subnet: config::Subnet,
    manager: Box<dyn SubnetManager + 'static>,
}

impl Connection {
    /// Get the subnet config.
    pub fn subnet(&self) -> &config::Subnet {
        &self.subnet
    }

    /// Get the subnet manager instance.
    pub fn manager(&self) -> &dyn SubnetManager {
        self.manager.borrow()
    }
}

#[derive(Clone)]
pub struct IpcProvider {
    sender: Option<Address>,
    config: Arc<Config>,
    fvm_wallet: Option<Arc<RwLock<Wallet>>>,
    evm_keystore: Option<Arc<RwLock<PersistentKeyStore<EthKeyAddress>>>>,
}

impl IpcProvider {
    fn new(
        config: Arc<Config>,
        fvm_wallet: Arc<RwLock<Wallet>>,
        evm_keystore: Arc<RwLock<PersistentKeyStore<EthKeyAddress>>>,
    ) -> Self {
        Self {
            sender: None,
            config,
            fvm_wallet: Some(fvm_wallet),
            evm_keystore: Some(evm_keystore),
        }
    }

    /// Initializes an `IpcProvider` from the config specified in the
    /// argument's config path.
    pub fn new_from_config(config_path: String) -> anyhow::Result<Self> {
        let config = Arc::new(Config::from_file(config_path)?);
        let fvm_wallet = Arc::new(RwLock::new(Wallet::new(new_fvm_wallet_from_config(
            config.clone(),
        )?)));
        let evm_keystore = Arc::new(RwLock::new(new_evm_keystore_from_config(config.clone())?));
        Ok(Self::new(config, fvm_wallet, evm_keystore))
    }

    /// Initializes a new `IpcProvider` configured to interact with
    /// a single subnet.
    pub fn new_with_subnet(
        keystore_path: Option<String>,
        subnet: config::Subnet,
    ) -> anyhow::Result<Self> {
        let mut config = Config::new();
        config.add_subnet(subnet);
        let config = Arc::new(config);

        if let Some(repo_path) = keystore_path {
            let fvm_wallet = Arc::new(RwLock::new(Wallet::new(new_fvm_keystore_from_path(
                &repo_path,
            )?)));
            let evm_keystore = Arc::new(RwLock::new(new_evm_keystore_from_path(&repo_path)?));
            Ok(Self::new(config, fvm_wallet, evm_keystore))
        } else {
            Ok(Self {
                sender: None,
                config,
                fvm_wallet: None,
                evm_keystore: None,
            })
        }
    }

    /// Initialized an `IpcProvider` using the default config path.
    pub fn new_default() -> anyhow::Result<Self> {
        Self::new_from_config(default_config_path())
    }

    /// Get the connection instance for the subnet.
    pub fn connection(&self, subnet: &SubnetID) -> Option<Connection> {
        let subnets = &self.config.subnets;
        match subnets.get(subnet) {
            Some(subnet) => match &subnet.config {
                config::subnet::SubnetConfig::Fevm(_) => {
                    let wallet = match self.evm_wallet() {
                        Ok(w) => Some(w),
                        Err(e) => {
                            log::warn!("error initializing evm wallet: {e}");
                            None
                        }
                    };
                    let manager =
                        match EthSubnetManager::from_subnet_with_wallet_store(subnet, wallet) {
                            Ok(w) => Some(w),
                            Err(e) => {
                                log::warn!("error initializing evm wallet: {e}");
                                return None;
                            }
                        };
                    Some(Connection {
                        manager: Box::new(manager.unwrap()),
                        subnet: subnet.clone(),
                    })
                }
            },
            None => None,
        }
    }

    /// Set the default account for the provider
    pub fn with_sender(&mut self, from: Address) {
        self.sender = Some(from);
    }

    /// Returns the evm wallet if it is configured, and throws an error if no wallet configured.
    ///
    /// This method should be used when we want the wallet retrieval to throw an error
    /// if it is not configured (i.e. when the provider needs to sign transactions).
    pub fn evm_wallet(&self) -> anyhow::Result<Arc<RwLock<PersistentKeyStore<EthKeyAddress>>>> {
        if let Some(wallet) = &self.evm_keystore {
            Ok(wallet.clone())
        } else {
            Err(anyhow!("No evm wallet found in provider"))
        }
    }

    // FIXME: Reconcile these into a single wallet method that
    // accepts an `ipc_identity::WalletType` as an input.
    pub fn fvm_wallet(&self) -> anyhow::Result<Arc<RwLock<Wallet>>> {
        if let Some(wallet) = &self.fvm_wallet {
            Ok(wallet.clone())
        } else {
            Err(anyhow!("No fvm wallet found in provider"))
        }
    }

    fn check_sender(
        &mut self,
        subnet: &config::Subnet,
        from: Option<Address>,
    ) -> anyhow::Result<Address> {
        // if there is from use that.
        if let Some(from) = from {
            return Ok(from);
        }

        // if not use the sender.
        if let Some(sender) = self.sender {
            return Ok(sender);
        }

        // and finally, if there is no sender, use the default and
        // set it as the default sender.
        match &subnet.config {
            config::subnet::SubnetConfig::Fevm(_) => {
                if self.sender.is_none() {
                    let wallet = self.evm_wallet()?;
                    let addr = match wallet.write().unwrap().get_default()? {
                        None => return Err(anyhow!("no default evm account configured")),
                        Some(addr) => Address::try_from(addr)?,
                    };
                    self.sender = Some(addr);
                    return Ok(addr);
                }
            }
        };

        Err(anyhow!("error fetching a valid sender"))
    }

    /// Lists available subnet connections
    pub fn list_connections(&self) -> HashMap<SubnetID, config::Subnet> {
        self.config.subnets.clone()
    }
}

/// IpcProvider spawns a daemon-less client to interact with IPC subnets.
///
/// At this point the provider assumes that the user providers a `config.toml`
/// with the subnet configuration. This has been inherited by the daemon
/// configuration and will be slowly deprecated.
impl IpcProvider {
    // FIXME: Once the arguments for subnet creation are stabilized,
    // use a SubnetOpts struct to provide the creation arguments and
    // remove this allow
    #[allow(clippy::too_many_arguments)]
    pub async fn create_subnet(
        &mut self,
        from: Option<Address>,
        parent: SubnetID,
        min_validators: u64,
        min_validator_stake: TokenAmount,
        bottomup_check_period: ChainEpoch,
        active_validators_limit: u16,
        min_cross_msg_fee: TokenAmount,
        permissioned: bool,
    ) -> anyhow::Result<Address> {
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        let constructor_params = ConstructParams {
            parent,
            ipc_gateway_addr: subnet_config.gateway_addr(),
            consensus: ConsensusType::Fendermint,
            min_validators,
            min_validator_stake,
            bottomup_check_period,
            active_validators_limit,
            min_cross_msg_fee,
            permissioned,
        };

        conn.manager()
            .create_subnet(sender, constructor_params)
            .await
    }

    pub async fn join_subnet(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
        collateral: TokenAmount,
        public_key: Vec<u8>,
    ) -> anyhow::Result<ChainEpoch> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager()
            .join_subnet(subnet, sender, collateral, public_key)
            .await
    }

    pub async fn pre_fund(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
        balance: TokenAmount,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().pre_fund(subnet, sender, balance).await
    }

    pub async fn pre_release(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
        amount: TokenAmount,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().pre_release(subnet, sender, amount).await
    }

    pub async fn stake(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
        collateral: TokenAmount,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().stake(subnet, sender, collateral).await
    }

    pub async fn unstake(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
        collateral: TokenAmount,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().unstake(subnet, sender, collateral).await
    }

    pub async fn leave_subnet(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().leave_subnet(subnet, sender).await
    }

    pub async fn claim_collateral(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().claim_collateral(subnet, sender).await
    }

    pub async fn claim_relayer_reward(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().claim_relayer_reward(subnet, sender).await
    }

    pub async fn kill_subnet(
        &mut self,
        subnet: SubnetID,
        from: Option<Address>,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager().kill_subnet(subnet, sender).await
    }

    pub async fn list_child_subnets(
        &self,
        gateway_addr: Option<Address>,
        subnet: &SubnetID,
    ) -> anyhow::Result<HashMap<SubnetID, SubnetInfo>> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();

        let gateway_addr = match gateway_addr {
            None => subnet_config.gateway_addr(),
            Some(addr) => addr,
        };

        conn.manager().list_child_subnets(gateway_addr).await
    }

    /// Funds an account in a child subnet, if `to` is `None`, the self account
    /// is funded.
    pub async fn fund(
        &mut self,
        subnet: SubnetID,
        gateway_addr: Option<Address>,
        from: Option<Address>,
        to: Option<Address>,
        amount: TokenAmount,
    ) -> anyhow::Result<ChainEpoch> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        let gateway_addr = match gateway_addr {
            None => subnet_config.gateway_addr(),
            Some(addr) => addr,
        };

        conn.manager()
            .fund(subnet, gateway_addr, sender, to.unwrap_or(sender), amount)
            .await
    }

    /// Release to an account in a child subnet, if `to` is `None`, the self account
    /// is funded.
    pub async fn release(
        &mut self,
        subnet: SubnetID,
        gateway_addr: Option<Address>,
        from: Option<Address>,
        to: Option<Address>,
        amount: TokenAmount,
        fee: Option<TokenAmount>,
    ) -> anyhow::Result<ChainEpoch> {
        let conn = match self.connection(&subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        let gateway_addr = match gateway_addr {
            None => subnet_config.gateway_addr(),
            Some(addr) => addr,
        };

        conn.manager()
            .release(gateway_addr, sender, to.unwrap_or(sender), amount, fee)
            .await
    }

    /// Propagate a cross-net message forward. For `postbox_msg_key`, we are using bytes because different
    /// runtime have different representations. For FVM, it should be `CID` as bytes. For EVM, it is
    /// `bytes32`.
    pub async fn propagate(
        &self,
        _subnet: SubnetID,
        _gateway_addr: Address,
        _from: Address,
        _postbox_msg_key: Vec<u8>,
    ) -> anyhow::Result<()> {
        todo!()
    }

    pub async fn send_cross_message(
        &self,
        _gateway_addr: Address,
        _from: Address,
        _cross_msg: CrossMsg,
    ) -> anyhow::Result<()> {
        todo!()
    }

    /// Send value between two addresses in a subnet
    pub async fn send_value(
        &mut self,
        subnet: &SubnetID,
        from: Option<Address>,
        to: Address,
        amount: TokenAmount,
    ) -> anyhow::Result<()> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        // FIXME: This limits that only value to f-addresses can be sent
        // with the provider (which requires translating eth-addresses into
        // their corresponding delegated address). This should be fixed with the
        // new address wrapper type planned: https://github.com/consensus-shipyard/ipc-agent/issues/263
        // let to = match Address::from_str(&request.to) {
        //     Ok(addr) => addr,
        //     Err(_) => {
        //         // we need to check if an 0x address was passed and convert
        //         // to a delegated address
        //         ethers_address_to_fil_address(&ethers::types::Address::from_str(&request.to)?)?
        //     }
        // };

        conn.manager().send_value(sender, to, amount).await
    }

    /// Get the balance of an address
    pub async fn wallet_balance(
        &self,
        subnet: &SubnetID,
        address: &Address,
    ) -> anyhow::Result<TokenAmount> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().wallet_balance(address).await
    }

    pub async fn chain_head(&self, subnet: &SubnetID) -> anyhow::Result<ChainEpoch> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().chain_head_height().await
    }

    /// Obtain the genesis epoch of the input subnet.
    pub async fn genesis_epoch(&self, subnet: &SubnetID) -> anyhow::Result<ChainEpoch> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("parent subnet config not found")),
            Some(conn) => conn,
        };
        conn.manager().genesis_epoch(subnet).await
    }

    /// Get the validator information.
    pub async fn get_validator_info(
        &self,
        subnet: &SubnetID,
        validator: &Address,
    ) -> anyhow::Result<ValidatorInfo> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target subnet parent not found")),
            Some(conn) => conn,
        };

        conn.manager().get_validator_info(subnet, validator).await
    }

    /// Get the changes in subnet validators. This is fetched from parent.
    pub async fn get_validator_changeset(
        &self,
        subnet: &SubnetID,
        epoch: ChainEpoch,
    ) -> anyhow::Result<TopDownQueryPayload<Vec<StakingChangeRequest>>> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target subnet parent not found")),
            Some(conn) => conn,
        };

        conn.manager().get_validator_changeset(subnet, epoch).await
    }

    /// Get genesis info for a child subnet. This can be used to deterministically
    /// generate the genesis of the subnet
    pub async fn get_genesis_info(&self, subnet: &SubnetID) -> anyhow::Result<SubnetGenesisInfo> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("parent subnet config not found")),
            Some(conn) => conn,
        };
        conn.manager().get_genesis_info(subnet).await
    }

    pub async fn get_top_down_msgs(
        &self,
        subnet: &SubnetID,
        epoch: ChainEpoch,
    ) -> anyhow::Result<TopDownQueryPayload<Vec<CrossMsg>>> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().get_top_down_msgs(subnet, epoch).await
    }

    pub async fn get_block_hash(
        &self,
        subnet: &SubnetID,
        height: ChainEpoch,
    ) -> anyhow::Result<GetBlockHashResult> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().get_block_hash(height).await
    }

    pub async fn get_chain_id(&self, subnet: &SubnetID) -> anyhow::Result<String> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().get_chain_id().await
    }

    pub async fn get_chain_head_height(&self, subnet: &SubnetID) -> anyhow::Result<ChainEpoch> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().chain_head_height().await
    }

    pub async fn get_bottom_up_bundle(
        &self,
        subnet: &SubnetID,
        height: ChainEpoch,
    ) -> anyhow::Result<BottomUpCheckpointBundle> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().checkpoint_bundle_at(height).await
    }

    pub async fn has_submitted_in_last_checkpoint_height(
        &self,
        subnet: &SubnetID,
        addr: &Address,
    ) -> anyhow::Result<bool> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("parent subnet not found")),
            Some(conn) => conn,
        };

        conn.manager()
            .has_submitted_in_last_checkpoint_height(subnet, addr)
            .await
    }

    pub async fn last_bottom_up_checkpoint_height(
        &self,
        subnet: &SubnetID,
    ) -> anyhow::Result<ChainEpoch> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("parent subnet not found")),
            Some(conn) => conn,
        };

        conn.manager()
            .last_bottom_up_checkpoint_height(subnet)
            .await
    }

    pub async fn quorum_reached_events(
        &self,
        subnet: &SubnetID,
        height: ChainEpoch,
    ) -> anyhow::Result<Vec<QuorumReachedEvent>> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().quorum_reached_events(height).await
    }

    /// Advertises the endpoint of a bootstrap node for the subnet.
    pub async fn add_bootstrap(
        &mut self,
        subnet: &SubnetID,
        from: Option<Address>,
        endpoint: String,
    ) -> anyhow::Result<()> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        let subnet_config = conn.subnet();
        let sender = self.check_sender(subnet_config, from)?;

        conn.manager()
            .add_bootstrap(subnet, &sender, endpoint)
            .await
    }

    /// Lists the bootstrap nodes of a subnet
    pub async fn list_bootstrap_nodes(&self, subnet: &SubnetID) -> anyhow::Result<Vec<String>> {
        let parent = subnet.parent().ok_or_else(|| anyhow!("no parent found"))?;
        let conn = match self.connection(&parent) {
            None => return Err(anyhow!("target parent subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().list_bootstrap_nodes(subnet).await
    }

    /// Returns the latest finality from the parent committed in a child subnet.
    pub async fn latest_parent_finality(&self, subnet: &SubnetID) -> anyhow::Result<ChainEpoch> {
        let conn = match self.connection(subnet) {
            None => return Err(anyhow!("target subnet not found")),
            Some(conn) => conn,
        };

        conn.manager().latest_parent_finality().await
    }
}

/// Lotus JSON keytype format
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LotusJsonKeyType {
    pub r#type: String,
    pub private_key: String,
}

impl FromStr for LotusJsonKeyType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v = serde_json::from_str(s)?;
        Ok(v)
    }
}

impl Drop for LotusJsonKeyType {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

// Here I put in some other category the wallet-related
// function so we can reconcile them easily when we decide to tackle
// https://github.com/consensus-shipyard/ipc-agent/issues/308
// This should become its own module within the provider, we should have different
// categories for each group of commands
impl IpcProvider {
    pub fn new_fvm_key(&self, tp: WalletKeyType) -> anyhow::Result<Address> {
        let tp = match tp {
            WalletKeyType::BLS => SignatureType::BLS,
            WalletKeyType::Secp256k1 => SignatureType::Secp256k1,
            WalletKeyType::Secp256k1Ledger => return Err(anyhow!("ledger key type not supported")),
        };

        self.fvm_wallet()?.write().unwrap().generate_addr(tp)
    }

    pub fn new_evm_key(&self) -> anyhow::Result<EthKeyAddress> {
        let key_info = ipc_identity::random_eth_key_info();
        let wallet = self.evm_wallet()?;

        let out = wallet.write().unwrap().put(key_info);
        out
    }

    pub fn import_fvm_key(&self, keyinfo: String) -> anyhow::Result<Address> {
        let wallet = self.fvm_wallet()?;
        let mut wallet = wallet.write().unwrap();
        let keyinfo = LotusJsonKeyType::from_str(&keyinfo)?;

        let key_type = if WalletKeyType::from_str(&keyinfo.r#type)? == WalletKeyType::BLS {
            SignatureType::BLS
        } else {
            SignatureType::Secp256k1
        };

        let key_info = ipc_identity::json::KeyInfoJson(ipc_identity::KeyInfo::new(
            key_type,
            base64::engine::general_purpose::STANDARD.decode(&keyinfo.private_key)?,
        ));
        let key_info = ipc_identity::KeyInfo::try_from(key_info)
            .map_err(|_| anyhow!("couldn't get fvm key info from string"))?;
        Ok(wallet.import(key_info)?)
    }

    pub fn import_evm_key_from_privkey(
        &self,
        private_key: String,
    ) -> anyhow::Result<EthKeyAddress> {
        let keystore = self.evm_wallet()?;
        let mut keystore = keystore.write().unwrap();

        let private_key = if !private_key.starts_with("0x") {
            hex::decode(&private_key)?
        } else {
            hex::decode(&private_key.as_str()[2..])?
        };
        keystore.put(ipc_identity::EvmKeyInfo::new(private_key))
    }

    pub fn import_evm_key_from_json(&self, keyinfo: String) -> anyhow::Result<EthKeyAddress> {
        let persisted: ipc_identity::PersistentKeyInfo = serde_json::from_str(&keyinfo)?;
        self.import_evm_key_from_privkey(persisted.private_key().parse()?)
    }
}

fn new_fvm_wallet_from_config(config: Arc<Config>) -> anyhow::Result<KeyStore> {
    let repo_str = &config.keystore_path;
    if let Some(repo_str) = repo_str {
        new_fvm_keystore_from_path(repo_str)
    } else {
        Err(anyhow!(
            "No keystore repo found in config. Try using absolute path"
        ))
    }
}

pub fn new_evm_keystore_from_config(
    config: Arc<Config>,
) -> anyhow::Result<PersistentKeyStore<EthKeyAddress>> {
    let repo_str = &config.keystore_path;
    if let Some(repo_str) = repo_str {
        new_evm_keystore_from_path(repo_str)
    } else {
        Err(anyhow!("No keystore repo found in config"))
    }
}

pub fn new_evm_keystore_from_path(
    repo_str: &str,
) -> anyhow::Result<PersistentKeyStore<EthKeyAddress>> {
    let repo = Path::new(&repo_str).join(ipc_identity::DEFAULT_KEYSTORE_NAME);
    let repo = expand_tilde(repo);
    PersistentKeyStore::new(repo).map_err(|e| anyhow!("Failed to create evm keystore: {}", e))
}

pub fn new_fvm_keystore_from_path(repo_str: &str) -> anyhow::Result<KeyStore> {
    let repo = Path::new(&repo_str);
    let repo = expand_tilde(repo);
    let keystore_config = KeyStoreConfig::Persistent(repo);
    // TODO: we currently only support persistent keystore in the default repo directory.
    KeyStore::new(keystore_config).map_err(|e| anyhow!("Failed to create keystore: {}", e))
}

pub fn default_repo_path() -> String {
    let home = match std::env::var("HOME") {
        Ok(home) => home,
        Err(_) => panic!("cannot get home"),
    };
    format!("{home:}/{:}", DEFAULT_REPO_PATH)
}

pub fn default_config_path() -> String {
    format!("{}/{:}", default_repo_path(), DEFAULT_CONFIG_NAME)
}

/// Expand paths that begin with "~" to `$HOME`.
pub fn expand_tilde<P: AsRef<Path>>(path: P) -> PathBuf {
    let p = path.as_ref().to_path_buf();
    if !p.starts_with("~") {
        return p;
    }
    if p == Path::new("~") {
        return dirs::home_dir().unwrap_or(p);
    }
    dirs::home_dir()
        .map(|mut h| {
            if h == Path::new("/") {
                // `~/foo` becomes just `/foo` instead of `//foo` if `/` is home.
                p.strip_prefix("~").unwrap().to_path_buf()
            } else {
                h.push(p.strip_prefix("~/").unwrap());
                h
            }
        })
        .unwrap_or(p)
}

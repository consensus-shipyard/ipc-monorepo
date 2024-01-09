// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
//! Setup infra for integration testing

pub mod subnet;
pub mod util;

use crate::infra::subnet::{spawn_first_node, spawn_other_nodes, SubnetNode};
use crate::infra::util::trim_newline;
use anyhow::anyhow;
use fvm_shared::address::Address;
use ipc_agent::config::subnet::FVMSubnet;
use ipc_agent::config::{Config, Subnet};
use ipc_api::subnet_id::SubnetID;
use std::str::FromStr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::thread::sleep;

const DEFAULT_IPC_AGENT_URL: &str = "http://localhost:3030/json_rpc";
const DEFAULT_NODE_API_BASE_PORT: u16 = 1230;
const DEFAULT_MIN_STAKE: f64 = 1.0;

/// The configuration struct for the subnet to spawn
pub struct SubnetConfig {
    /// The id of the subnet. If not specified, will create a subnet first.
    pub id: Option<SubnetID>,
    /// Name of the subnet
    pub name: String,
    /// The parent of the subnet
    pub parent: SubnetID,
    /// Number of nodes in the subnet
    pub number_of_nodes: usize,
    /// The path to eudico binary. Since most of the operations are issued from
    /// command line, we need to point to the eudico binary path.
    pub eudico_binary_path: String,
    /// The parent subnet wallet address. This will be used to perform setups in the parent
    /// subnet, such as initial fund transfer to the validators so that validators can join
    /// the created subnet
    pub parent_wallet_address: String,
    /// The parent subnet eudico lotus path
    pub parent_lotus_path: String,
    /// The ipc agent root folder
    pub ipc_root_folder: String,

    ipc_agent_url: Option<String>,

    /// The monotonic sequential port number generator to assign to each validator
    port_starting_seq: Arc<AtomicU16>,
}

impl SubnetConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        parent_wallet_address: String,
        parent_lotus_path: String,
        ipc_root_folder: String,
        number_of_nodes: usize,
        eudico_binary_path: String,
        parent: SubnetID,
        port_starting_seq: Arc<AtomicU16>,
    ) -> Self {
        Self {
            id: None,
            name,
            number_of_nodes,
            eudico_binary_path,
            parent,
            parent_wallet_address,
            parent_lotus_path,
            ipc_root_folder,
            port_starting_seq,
            ipc_agent_url: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_subnet_id(
        name: String,
        parent_wallet_address: String,
        parent_lotus_path: String,
        ipc_root_folder: String,
        number_of_nodes: usize,
        eudico_binary_path: String,
        parent: SubnetID,
        port_starting_seq: Arc<AtomicU16>,
        id: SubnetID,
    ) -> Self {
        Self {
            id: Some(id),
            name,
            number_of_nodes,
            eudico_binary_path,
            parent,
            parent_wallet_address,
            parent_lotus_path,
            ipc_root_folder,
            port_starting_seq,
            ipc_agent_url: None,
        }
    }

    pub fn ipc_agent_url(&self) -> String {
        self.ipc_agent_url
            .clone()
            .unwrap_or_else(|| DEFAULT_IPC_AGENT_URL.to_string())
    }

    pub fn next_port(&self) -> u16 {
        loop {
            let r = self.port_starting_seq.load(Ordering::SeqCst);
            if self
                .port_starting_seq
                .compare_exchange(r, r + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return r + DEFAULT_NODE_API_BASE_PORT;
            }
        }
    }
}

pub struct SubnetInfra {
    pub config: SubnetConfig,
    pub nodes: Option<Vec<SubnetNode>>,
}

impl SubnetInfra {
    pub fn new(config: SubnetConfig) -> Self {
        Self {
            config,
            nodes: None,
        }
    }

    pub async fn create_subnet(&mut self) -> anyhow::Result<()> {
        let parent = self.config.parent.to_string();

        let actor_addr = util::create_subnet(
            self.config.ipc_agent_url(),
            None,
            parent,
            self.config.name.clone(),
            self.config.number_of_nodes as u64,
        )
        .await?;

        self.config.id = Some(SubnetID::new_from_parent(
            &self.config.parent,
            Address::from_str(&actor_addr)?,
        ));

        Ok(())
    }

    pub fn start_nodes(&mut self) -> anyhow::Result<()> {
        let first_node = spawn_first_node(&self.config)?;
        let mut nodes = spawn_other_nodes(&self.config, &first_node)?;

        nodes.push(first_node);

        self.nodes = Some(nodes);

        Ok(())
    }

    pub async fn start_validators(&mut self) -> anyhow::Result<()> {
        if self.nodes.is_none() {
            return Err(anyhow!("nodes not spawned yet"));
        }

        for node in self.nodes.as_mut().unwrap() {
            node.config_validator()?;
            log::info!(
                "configured validator for node: {:?}",
                node.validator.net_addr
            );

            node.export_wallet_to_ipc_key_store().await?;
            node.join_subnet().await?;
            log::info!(
                "validator: {:?} joined subnet: {:}",
                node.validator.net_addr,
                node.id
            );

            sleep(std::time::Duration::from_secs(5));

            node.spawn_validator()?;

            log::info!("validator: {:?} spawned", node.validator.net_addr);
        }

        Ok(())
    }

    pub fn fund_node_wallets(&mut self) -> anyhow::Result<()> {
        if self.nodes.is_none() {
            return Err(anyhow!("no nodes launched"));
        }

        util::fund_wallet_in_nodes(
            &self.config.eudico_binary_path,
            &self.config.parent_lotus_path,
            self.nodes.as_ref().unwrap(),
            10,
        )?;

        Ok(())
    }

    /// Tear down all nodes and validators
    pub fn tear_down(&mut self) -> anyhow::Result<()> {
        if self.nodes.is_none() {
            return Err(anyhow!("no nodes launched"));
        }

        let nodes = self.nodes.take().unwrap();
        drop(nodes);

        Ok(())
    }

    /// Add subnet info to ipc agent config
    pub async fn update_ipc_agent_config(&self) -> anyhow::Result<()> {
        if self.config.id.is_none() {
            return Err(anyhow!("subnet id not set"));
        }

        let config_path = self.ipc_config_path();
        let mut config = Config::from_file(&config_path)
            .map_err(|e| anyhow!("cannot load config from {config_path:?} due to: {e:}"))?;
        log::debug!("loaded config: {config:?}");

        let subnet = self.subnet_config().await?;
        log::debug!("created subnet config: {subnet:?}");

        config.add_subnet(subnet);
        config.write_to_file_async(&config_path).await
    }

    /// Remove subnet info from ipc agent config
    pub async fn remove_from_ipc_agent_config(&self) -> anyhow::Result<()> {
        if self.config.id.is_none() {
            return Err(anyhow!("subnet id not set"));
        }

        let config_path = self.ipc_config_path();
        let mut config = Config::from_file(&config_path)?;

        config.remove_subnet(self.config.id.as_ref().unwrap());
        config.write_to_file_async(&config_path).await?;

        Ok(())
    }

    pub async fn trigger_ipc_config_reload(&self) -> anyhow::Result<()> {
        let config_path = self.ipc_config_path();
        util::reload_config(self.config.ipc_agent_url(), Some(config_path)).await
    }

    fn ipc_config_path(&self) -> String {
        format!("{:}/{:}", self.config.ipc_root_folder, "config.toml")
    }

    async fn subnet_config(&self) -> anyhow::Result<Subnet> {
        if self.nodes.is_none() {
            return Err(anyhow!("nodes not up"));
        }
        if self.config.id.is_none() {
            return Err(anyhow!("subnet id not set"));
        }

        let accounts = self
            .nodes
            .as_ref()
            .unwrap()
            .iter()
            .map(|n| {
                if n.wallet_address.is_none() {
                    return Err(anyhow!("node wallet node setup"));
                }
                let address = Address::from_str(n.wallet_address.as_ref().unwrap())?;
                Ok(address)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut admin_token = self.nodes.as_ref().unwrap()[0].create_admin_token().await?;
        trim_newline(&mut admin_token);

        Ok(Subnet {
            id: self.config.id.clone().unwrap(),
            network_name: self.config.name.clone(),
            config: ipc_agent::config::subnet::SubnetConfig::Fvm(FVMSubnet {
                gateway_addr: Address::from_str("t064")?,
                jsonrpc_api_http: format!(
                    "http://127.0.0.1:{:}/rpc/v1",
                    self.nodes.as_ref().unwrap()[0].node.tcp_port
                )
                .parse()?,
                auth_token: Some(admin_token),
                accounts,
            }),
        })
    }
}

// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

use crate::infra::subnet::SubnetNode;
use crate::infra::DEFAULT_MIN_STAKE;
use anyhow::anyhow;
use ipc_agent::sdk::{IpcAgentClient, LotusJsonKeyType};
use ipc_agent::server::create::CreateSubnetParams;
use ipc_agent::server::join::JoinSubnetParams;
use ipc_provider::jsonrpc::JsonRpcClientImpl;
use std::process::Command;
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

fn client_from_url(url: String) -> anyhow::Result<IpcAgentClient<JsonRpcClientImpl>> {
    let url = url.parse()?;
    Ok(IpcAgentClient::default_from_url(url))
}

/// Create a new subnet in the actor
pub async fn create_subnet(
    ipc_agent_url: String,
    from: Option<String>,
    parent: String,
    name: String,
    min_validators: u64,
) -> anyhow::Result<String> {
    let client = client_from_url(ipc_agent_url)?;
    let params = CreateSubnetParams {
        from,
        parent,
        name,
        min_validator_stake: DEFAULT_MIN_STAKE,
        min_validators,
        bottomup_check_period: 10,
        topdown_check_period: 10,
    };
    client.create_subnet(params).await
}

/// Join the subnet
pub async fn join_subnet(
    ipc_agent_url: String,
    from: Option<String>,
    worker_addr: Option<String>,
    subnet: String,
    collateral: f64,
    validator_net_addr: String,
) -> anyhow::Result<()> {
    let client = client_from_url(ipc_agent_url)?;
    let params = JoinSubnetParams {
        subnet,
        from,
        collateral,
        validator_net_addr,
        worker_addr,
    };
    client.join_subnet(params).await
}

pub async fn reload_config(
    ipc_agent_url: String,
    config_path: Option<String>,
) -> anyhow::Result<()> {
    let client = client_from_url(ipc_agent_url)?;
    client.reload_config(config_path).await
}

/// Send token to the target address. Not that the `from` wallet address is not specified as it is
/// implied from the `lotus_path`.
pub fn send_token(
    eudico_binary_path: &str,
    lotus_path: &str,
    addr: &str,
    amount: u8,
) -> anyhow::Result<()> {
    let status = Command::new(eudico_binary_path)
        .args(["send", addr, &amount.to_string()])
        .env("LOTUS_PATH", lotus_path)
        .status()?;

    if status.success() {
        log::info!("funded wallet: {:} with amount: {:} fvm", addr, amount);
        Ok(())
    } else {
        Err(anyhow!("cannot send token to wallet:{:}", addr))
    }
}

/// Fund the wallet addresses associated with the nodes
pub fn fund_wallet_in_nodes(
    eudico_binary_path: &str,
    lotus_path: &str,
    nodes: &[SubnetNode],
    amount: u8,
) -> anyhow::Result<()> {
    for node in nodes.iter() {
        send_token(
            eudico_binary_path,
            lotus_path,
            node.wallet_address.as_ref().unwrap(),
            amount,
        )?;
        // for nonce to be updated
        sleep(Duration::from_secs(5));
    }
    Ok(())
}

/// Create a new wallet address for the node
pub fn create_wallet(node: &mut SubnetNode) -> anyhow::Result<()> {
    loop {
        match node.new_wallet_address() {
            Ok(_) => {
                log::info!("one wallet created in node: {:?}", node.id);
                break;
            }
            Err(e) => {
                log::warn!("cannot create wallet: {e:}, wait and sleep to retry");
                sleep(Duration::from_secs(10))
            }
        }
    }

    Ok(())
}

pub async fn import_wallet(ipc_agent_url: &str, private_key: String) -> anyhow::Result<()> {
    let params = LotusJsonKeyType::from_str(&private_key)?;
    let client = client_from_url(ipc_agent_url.to_string())?;
    client.import_lotus_json(params).await?;
    Ok(())
}

/// Filter and get the tcp address, input must contain tcp address
pub fn tcp_address(addrs: Vec<String>) -> anyhow::Result<String> {
    addrs
        .into_iter()
        .find(|a| a.contains("tcp"))
        .ok_or_else(|| anyhow!("no tcp address found"))
}

pub fn trim_newline(s: &mut String) {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
}

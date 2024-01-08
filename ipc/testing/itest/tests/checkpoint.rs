// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

use ipc_agent::sdk::IpcAgentClient;
use std::thread::sleep;
use std::time::Duration;

const IPC_AGENT_JSON_RPC_URL_ENV: &str = "IPC_AGENT_JSON_RPC_URL";
const CHILD_SUBNET_ID_STR_ENV: &str = "CHILD_SUBNET_ID_STR";
const FUND_ADDRESS_ENV: &str = "FUND_ADDRESS";

/// Checks the checkpoints are submitted
#[tokio::test]
async fn verify_checkpoints_submitted() {
    let url = std::env::var(IPC_AGENT_JSON_RPC_URL_ENV)
        .unwrap()
        .parse()
        .unwrap();
    let subnet = std::env::var(CHILD_SUBNET_ID_STR_ENV).unwrap();

    let ipc_client = IpcAgentClient::default_from_url(url);

    let epoch = ipc_client.last_top_down_executed(&subnet).await.unwrap();
    assert!(epoch > 0, "no top down message executed yet");

    // at least get the first 10 epoches, this should be the very first bottome up checkpoints
    let checkpoints = ipc_client
        .list_bottom_up_checkpoints(&subnet, 0, 10)
        .await
        .unwrap();
    assert!(
        !checkpoints.is_empty(),
        "no bottom up checkpoints executed yet"
    );
}

/// Test fund and release across the parent and child subnets
#[tokio::test]
async fn test_fund_and_release() {
    let url = std::env::var(IPC_AGENT_JSON_RPC_URL_ENV)
        .unwrap()
        .parse()
        .unwrap();
    let ipc_client = IpcAgentClient::default_from_url(url);

    let subnet = std::env::var(CHILD_SUBNET_ID_STR_ENV).unwrap();
    let addr = std::env::var(FUND_ADDRESS_ENV).unwrap();
    let amount = 2.5;

    let fund_epoch = ipc_client
        .fund(&subnet, Some(addr.clone()), Some(addr.clone()), amount)
        .await
        .unwrap();
    println!("fund epoch: {fund_epoch:}");
    loop {
        let epoch = ipc_client.last_top_down_executed(&subnet).await.unwrap();
        if epoch > fund_epoch {
            println!("fund epoch reached: {fund_epoch:}");
            break;
        }
        sleep(Duration::from_secs(30));
    }

    let epoch = ipc_client
        .release(&subnet, Some(addr.clone()), Some(addr.clone()), amount)
        .await
        .unwrap();
    println!("release epoch: {epoch:}");
    loop {
        let checkpoints = ipc_client
            .list_bottom_up_checkpoints(&subnet, epoch, epoch)
            .await
            .unwrap();
        if !checkpoints.is_empty() {
            println!("released in epoch: {epoch:}");
            break;
        }
        sleep(Duration::from_secs(30));
    }
}

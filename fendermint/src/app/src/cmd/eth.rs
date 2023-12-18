// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use std::time::Duration;

use anyhow::Context;
use fendermint_rpc::client::ws_client;
use tendermint_rpc::{Url, WebSocketClient, WebSocketClientDriver};

use crate::{
    cmd,
    options::eth::{EthArgs, EthCommands},
    settings::eth::EthSettings,
};

cmd! {
  EthArgs(self, settings: EthSettings) {
    match self.command.clone() {
      EthCommands::Run { url, proxy_url:_, connect_max_retries, connect_retry_delay } => {

        let (client, driver) = ws_connect(url, connect_max_retries, Duration::from_secs(connect_retry_delay)).await.context("failed to connect to Tendermint")?;

        let driver_handle = tokio::spawn(async move { driver.run().await });

        let result = run(settings, client).await;

        // Await the driver's termination to ensure proper connection closure.
        let _ = driver_handle.await;
        result
      }
    }
  }
}

/// Run the Ethereum
async fn run(settings: EthSettings, client: WebSocketClient) -> anyhow::Result<()> {
    let gas = fendermint_eth_api::GasOpt {
        min_gas_premium: settings.gas.min_gas_premium,
        num_blocks_max_prio_fee: settings.gas.num_blocks_max_prio_fee,
        max_fee_hist_size: settings.gas.max_fee_hist_size,
    };
    fendermint_eth_api::listen(
        settings.listen,
        client,
        settings.filter_timeout,
        settings.cache_capacity,
        gas,
    )
    .await
}

/// Try connecting repeatedly until it succeeds.
async fn ws_connect(
    url: Url,
    max_retries: usize,
    retry_delay: Duration,
) -> anyhow::Result<(WebSocketClient, WebSocketClientDriver)> {
    let mut retry = 0;
    loop {
        match ws_client(url.clone()).await {
            Ok(cd) => {
                return Ok(cd);
            }
            Err(e) => {
                if retry >= max_retries {
                    return Err(e);
                } else {
                    tracing::warn!("failed to connect to Tendermint; retrying...");
                    retry += 1;
                    tokio::time::sleep(retry_delay).await;
                }
            }
        }
    }
}

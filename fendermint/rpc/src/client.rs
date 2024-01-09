// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use std::fmt::Display;
use std::marker::PhantomData;

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use fendermint_vm_message::chain::ChainMessage;
use tendermint::abci::response::DeliverTx;
use tendermint::block::Height;
use tendermint_rpc::{endpoint::abci_query::AbciQuery, Client, HttpClient, Scheme, Url};
use tendermint_rpc::{WebSocketClient, WebSocketClientDriver, WebSocketClientUrl};

use fendermint_vm_message::query::{FvmQuery, FvmQueryHeight};

use crate::message::MessageFactory;
use crate::query::QueryClient;
use crate::tx::{
    AsyncResponse, BoundClient, CommitResponse, SyncResponse, TxAsync, TxClient, TxCommit, TxSync,
};

// Retrieve the proxy URL with precedence:
// 1. If supplied, that's the proxy URL used.
// 2. If not supplied, but environment variable HTTP_PROXY or HTTPS_PROXY are
//    supplied, then use the appropriate variable for the URL in question.
//
// Copied from `tendermint_rpc`.
fn get_http_proxy_url(url_scheme: Scheme, proxy_url: Option<Url>) -> anyhow::Result<Option<Url>> {
    match proxy_url {
        Some(u) => Ok(Some(u)),
        None => match url_scheme {
            Scheme::Http => std::env::var("HTTP_PROXY").ok(),
            Scheme::Https => std::env::var("HTTPS_PROXY")
                .ok()
                .or_else(|| std::env::var("HTTP_PROXY").ok()),
            _ => {
                if std::env::var("HTTP_PROXY").is_ok() || std::env::var("HTTPS_PROXY").is_ok() {
                    tracing::warn!(
                        "Ignoring HTTP proxy environment variables for non-HTTP client connection"
                    );
                }
                None
            }
        }
        .map(|u| u.parse::<Url>().map_err(|e| anyhow!(e)))
        .transpose(),
    }
}

/// Create a Tendermint HTTP client.
pub fn http_client(url: Url, proxy_url: Option<Url>) -> anyhow::Result<HttpClient> {
    let proxy_url = get_http_proxy_url(url.scheme(), proxy_url)?;
    let client = match proxy_url {
        Some(proxy_url) => {
            tracing::debug!(
                "Using HTTP client with proxy {} to submit request to {}",
                proxy_url,
                url
            );
            HttpClient::new_with_proxy(url, proxy_url)?
        }
        None => {
            tracing::debug!("Using HTTP client to submit request to: {}", url);
            HttpClient::new(url)?
        }
    };
    Ok(client)
}

/// Create a Tendermint WebSocket client.
///
/// The caller must start the driver in a background task.
pub async fn ws_client<U>(url: U) -> anyhow::Result<(WebSocketClient, WebSocketClientDriver)>
where
    U: TryInto<WebSocketClientUrl, Error = tendermint_rpc::Error> + Display + Clone,
{
    // TODO: Doesn't handle proxy.
    tracing::debug!("Using WS client to submit request to: {}", url);

    let (client, driver) = WebSocketClient::new(url.clone())
        .await
        .with_context(|| format!("failed to create WS client to: {}", url))?;

    Ok((client, driver))
}

/// Unauthenticated Fendermint client.
#[derive(Clone)]
pub struct FendermintClient<C = HttpClient> {
    inner: C,
}

impl<C> FendermintClient<C> {
    pub fn new(inner: C) -> Self {
        Self { inner }
    }

    /// Attach a message factory to the client.
    pub fn bind(self, message_factory: MessageFactory) -> BoundFendermintClient<C> {
        BoundFendermintClient::new(self.inner, message_factory)
    }
}

impl FendermintClient<HttpClient> {
    pub fn new_http(url: Url, proxy_url: Option<Url>) -> anyhow::Result<Self> {
        let inner = http_client(url, proxy_url)?;
        Ok(Self { inner })
    }
}

/// Get to the underlying Tendermint client if necessary, for example to query the state of transactions.
pub trait TendermintClient<C> {
    /// The underlying Tendermint client.
    fn underlying(&self) -> &C;
}

impl<C> TendermintClient<C> for FendermintClient<C> {
    fn underlying(&self) -> &C {
        &self.inner
    }
}

#[async_trait]
impl<C> QueryClient for FendermintClient<C>
where
    C: Client + Sync + Send,
{
    async fn perform(&self, query: FvmQuery, height: FvmQueryHeight) -> anyhow::Result<AbciQuery> {
        perform_query(&self.inner, query, height).await
    }
}

/// Fendermint client capable of signing transactions.
pub struct BoundFendermintClient<C = HttpClient> {
    inner: C,
    message_factory: MessageFactory,
}

impl<C> BoundFendermintClient<C> {
    pub fn new(inner: C, message_factory: MessageFactory) -> Self {
        Self {
            inner,
            message_factory,
        }
    }
}

impl<C> BoundClient for BoundFendermintClient<C> {
    fn message_factory_mut(&mut self) -> &mut MessageFactory {
        &mut self.message_factory
    }
}

impl<C> TendermintClient<C> for BoundFendermintClient<C> {
    fn underlying(&self) -> &C {
        &self.inner
    }
}

#[async_trait]
impl<C> QueryClient for BoundFendermintClient<C>
where
    C: Client + Sync + Send,
{
    async fn perform(&self, query: FvmQuery, height: FvmQueryHeight) -> anyhow::Result<AbciQuery> {
        perform_query(&self.inner, query, height).await
    }
}

#[async_trait]
impl<C> TxClient<TxAsync> for BoundFendermintClient<C>
where
    C: Client + Sync + Send,
{
    async fn perform<F, T>(&self, msg: ChainMessage, _f: F) -> anyhow::Result<AsyncResponse<T>>
    where
        F: FnOnce(&DeliverTx) -> anyhow::Result<T> + Sync + Send,
    {
        let data = MessageFactory::serialize(&msg)?;
        let response = self.inner.broadcast_tx_async(data).await?;
        let response = AsyncResponse {
            response,
            return_data: PhantomData,
        };
        Ok(response)
    }
}

#[async_trait]
impl<C> TxClient<TxSync> for BoundFendermintClient<C>
where
    C: Client + Sync + Send,
{
    async fn perform<F, T>(
        &self,
        msg: ChainMessage,
        _f: F,
    ) -> anyhow::Result<crate::tx::SyncResponse<T>>
    where
        F: FnOnce(&DeliverTx) -> anyhow::Result<T> + Sync + Send,
    {
        let data = MessageFactory::serialize(&msg)?;
        let response = self.inner.broadcast_tx_sync(data).await?;
        let response = SyncResponse {
            response,
            return_data: PhantomData,
        };
        Ok(response)
    }
}

#[async_trait]
impl<C> TxClient<TxCommit> for BoundFendermintClient<C>
where
    C: Client + Sync + Send,
{
    async fn perform<F, T>(
        &self,
        msg: ChainMessage,
        f: F,
    ) -> anyhow::Result<crate::tx::CommitResponse<T>>
    where
        F: FnOnce(&DeliverTx) -> anyhow::Result<T> + Sync + Send,
    {
        let data = MessageFactory::serialize(&msg)?;
        let response = self.inner.broadcast_tx_commit(data).await?;
        // We have a fully `DeliverTx` with default fields even if `CheckTx` indicates failure.
        let return_data = if response.check_tx.code.is_err() || response.deliver_tx.code.is_err() {
            None
        } else {
            let return_data =
                f(&response.deliver_tx).context("error decoding data from deliver_tx in commit")?;
            Some(return_data)
        };
        let response = CommitResponse {
            response,
            return_data,
        };
        Ok(response)
    }
}

async fn perform_query<C>(
    client: &C,
    query: FvmQuery,
    height: FvmQueryHeight,
) -> anyhow::Result<AbciQuery>
where
    C: Client + Sync + Send,
{
    tracing::debug!(?query, ?height, "perform ABCI query");
    let data = fvm_ipld_encoding::to_vec(&query).context("failed to encode query")?;
    let height: u64 = height.into();
    let height = Height::try_from(height).context("failed to conver to Height")?;

    let res = client.abci_query(None, data, Some(height), false).await?;

    Ok(res)
}

// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::{anyhow, Context};
use config::{Config, ConfigError, Environment, File};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use ipc_api::subnet_id::SubnetID;
use serde::Deserialize;
use serde_with::{serde_as, DurationSeconds};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tendermint_rpc::Url;

use fendermint_vm_encoding::{human_readable_delegate, human_readable_str};
use fendermint_vm_topdown::BlockHeight;

use self::eth::EthSettings;
use self::fvm::FvmSettings;
use self::resolver::ResolverSettings;
use ipc_provider::config::deserialize::deserialize_eth_address_from_str;

pub mod eth;
pub mod fvm;
pub mod resolver;

/// Marker to be used with the `#[serde_as(as = "IsHumanReadable")]` annotations.
///
/// We can't just import `fendermint_vm_encoding::IsHumanReadable` because we can't implement traits for it here,
/// however we can use the `human_readable_delegate!` macro to delegate from this to that for the types we need
/// and it will look the same.
struct IsHumanReadable;

human_readable_str!(SubnetID);
human_readable_delegate!(TokenAmount);

#[derive(Debug, Deserialize, Clone)]
pub struct SocketAddress {
    pub host: String,
    pub port: u32,
}

impl ToString for SocketAddress {
    fn to_string(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl std::net::ToSocketAddrs for SocketAddress {
    type Iter = <String as std::net::ToSocketAddrs>::Iter;

    fn to_socket_addrs(&self) -> std::io::Result<Self::Iter> {
        self.to_string().to_socket_addrs()
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
/// Indicate the FVM account kind for generating addresses from a key.
pub enum AccountKind {
    /// Has an f1 address.
    Regular,
    /// Has an f410 address.
    Ethereum,
}

/// A Secp256k1 key used to sign transactions,
/// with the account kind showing if it's a regular or an ethereum key.
#[derive(Debug, Deserialize, Clone)]
pub struct SigningKey {
    path: PathBuf,
    pub kind: AccountKind,
}

home_relative!(SigningKey { path });

#[derive(Debug, Deserialize, Clone)]
pub struct AbciSettings {
    pub listen: SocketAddress,
    /// Queue size for each ABCI component.
    pub bound: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DbSettings {
    /// Length of the app state history to keep in the database before pruning; 0 means unlimited.
    ///
    /// This affects how long we can go back in state queries.
    pub state_hist_size: u64,
}

/// Settings affecting how we deal with failures in trying to send transactions to the local CometBFT node.
/// It is not expected to be unavailable, however we might get into race conditions about the nonce which
/// would need us to try creating a completely new transaction and try again.
#[serde_as]
#[derive(Debug, Deserialize, Clone)]
pub struct BroadcastSettings {
    /// Number of times to retry broadcasting a transaction.
    pub max_retries: u8,
    /// Time to wait between retries. This should roughly correspond to the block interval.
    #[serde_as(as = "DurationSeconds<u64>")]
    pub retry_delay: Duration,
    /// Any over-estimation to apply on top of the estimate returned by the API.
    pub gas_overestimation_rate: f64,
}

#[serde_as]
#[derive(Debug, Deserialize, Clone)]
pub struct TopDownSettings {
    /// The number of blocks to delay before reporting a height as final on the parent chain.
    /// To propose a certain number of epochs delayed from the latest height, we see to be
    /// conservative and avoid other from rejecting the proposal because they don't see the
    /// height as final yet.
    pub chain_head_delay: BlockHeight,
    /// The number of blocks on top of `chain_head_delay` to wait before proposing a height
    /// as final on the parent chain, to avoid slight disagreements between validators whether
    /// a block is final, or not just yet.
    pub proposal_delay: BlockHeight,
    /// The max number of blocks one should make the topdown proposal
    pub max_proposal_range: BlockHeight,
    /// Parent syncing cron period, in seconds
    #[serde_as(as = "DurationSeconds<u64>")]
    pub polling_interval: Duration,
    /// Top down exponential back off retry base
    #[serde_as(as = "DurationSeconds<u64>")]
    pub exponential_back_off: Duration,
    /// The max number of retries for exponential backoff before giving up
    pub exponential_retry_limit: usize,
    /// The parent rpc http endpoint
    pub parent_http_endpoint: Url,
    /// The parent registry address
    #[serde(deserialize_with = "deserialize_eth_address_from_str")]
    pub parent_registry: Address,
    /// The parent gateway address
    #[serde(deserialize_with = "deserialize_eth_address_from_str")]
    pub parent_gateway: Address,
}

#[serde_as]
#[derive(Debug, Deserialize, Clone)]
pub struct IpcSettings {
    #[serde_as(as = "IsHumanReadable")]
    pub subnet_id: SubnetID,
    /// The config for top down checkpoint. It's None if subnet id is root or not activating
    /// any top down checkpoint related operations
    pub topdown: Option<TopDownSettings>,
}

impl IpcSettings {
    pub fn is_topdown_enabled(&self) -> bool {
        !self.subnet_id.is_root() && self.topdown.is_some()
    }

    pub fn topdown_config(&self) -> anyhow::Result<&TopDownSettings> {
        self.topdown
            .as_ref()
            .ok_or_else(|| anyhow!("top down config missing"))
    }
}

#[serde_as]
#[derive(Debug, Deserialize, Clone)]
pub struct SnapshotSettings {
    /// Enable the export and import of snapshots.
    pub enabled: bool,
    /// How often to attempt to export snapshots in terms of block height.
    pub block_interval: BlockHeight,
    /// Number of snapshots to keep before purging old ones.
    pub hist_size: usize,
    /// Target chunk size, in bytes.
    pub chunk_size_bytes: usize,
    /// How long to keep a snapshot from being purged after it has been requested by a peer.
    #[serde_as(as = "DurationSeconds<u64>")]
    pub last_access_hold: Duration,
    /// How often to poll CometBFT to see whether it has caught up with the chain.
    #[serde_as(as = "DurationSeconds<u64>")]
    pub sync_poll_interval: Duration,
    /// Temporary directory for downloads.
    download_dir: Option<PathBuf>,
}

impl SnapshotSettings {
    pub fn download_dir(&self) -> PathBuf {
        self.download_dir.clone().unwrap_or(std::env::temp_dir())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    /// Home directory configured on the CLI, to which all paths in settings can be set relative.
    home_dir: PathBuf,
    /// Database files.
    data_dir: PathBuf,
    /// State snapshots.
    snapshots_dir: PathBuf,
    /// Solidity contracts.
    contracts_dir: PathBuf,
    /// Builtin-actors CAR file.
    builtin_actors_bundle: PathBuf,

    /// Where to reach CometBFT for queries or broadcasting transactions.
    tendermint_rpc_url: Url,

    /// Secp256k1 private key used for signing transactions sent in the validator's name. Leave empty if not validating.
    pub validator_key: Option<SigningKey>,

    pub abci: AbciSettings,
    pub db: DbSettings,
    pub snapshots: SnapshotSettings,
    pub eth: EthSettings,
    pub fvm: FvmSettings,
    pub resolver: ResolverSettings,
    pub broadcast: BroadcastSettings,
    pub ipc: IpcSettings,
}

#[macro_export]
macro_rules! home_relative {
    // Using this inside something that has a `.home_dir()` function.
    ($($name:ident),+) => {
        $(
        pub fn $name(&self) -> std::path::PathBuf {
            expand_path(&self.home_dir(), &self.$name)
        }
        )+
    };

    // Using this outside something that requires a `home_dir` parameter to be passed to it.
    ($settings:ty { $($name:ident),+ } ) => {
      impl $settings {
        $(
        pub fn $name(&self, home_dir: &std::path::Path) -> std::path::PathBuf {
            $crate::expand_path(home_dir, &self.$name)
        }
        )+
      }
    };
}

impl Settings {
    home_relative!(
        data_dir,
        snapshots_dir,
        contracts_dir,
        builtin_actors_bundle
    );

    /// Load the default configuration from a directory,
    /// then potential overrides specific to the run mode,
    /// then overrides from the local environment.
    pub fn new(config_dir: &Path, home_dir: &Path, run_mode: &str) -> Result<Self, ConfigError> {
        let c = Config::builder()
            .add_source(File::from(config_dir.join("default")))
            // Optional mode specific overrides, checked into git.
            .add_source(File::from(config_dir.join(run_mode)).required(false))
            // Optional local overrides, not checked into git.
            .add_source(File::from(config_dir.join("local")).required(false))
            // Add in settings from the environment (with a prefix of FM)
            // e.g. `FM_DB__DATA_DIR=./foo/bar ./target/app` would set the database location.
            .add_source(
                Environment::with_prefix("fm")
                    .prefix_separator("_")
                    .separator("__"),
            )
            // Set the home directory based on what was passed to the CLI,
            // so everything in the config can be relative to it.
            // The `home_dir` key is not added to `default.toml` so there is no confusion
            // about where it will be coming from.
            .set_override("home_dir", home_dir.to_string_lossy().as_ref())?
            .build()?;

        // Deserialize (and thus freeze) the entire configuration.
        c.try_deserialize()
    }

    /// The configured home directory.
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Tendermint RPC URL from the environment or the config file.
    pub fn tendermint_rpc_url(&self) -> anyhow::Result<Url> {
        // Prefer the "standard" env var used in the CLI.
        match std::env::var("TENDERMINT_RPC_URL").ok() {
            Some(url) => url.parse::<Url>().context("invalid Tendermint URL"),
            None => Ok(self.tendermint_rpc_url.clone()),
        }
    }
}

/// Expand a path which can either be :
/// * absolute, e.g. "/foo/bar"
/// * relative to the system `$HOME` directory, e.g. "~/foo/bar"
/// * relative to the configured `--home-dir` directory, e.g. "foo/bar"
pub fn expand_path(home_dir: &Path, path: &Path) -> PathBuf {
    if path.starts_with("/") {
        PathBuf::from(path)
    } else if path.starts_with("~") {
        expand_tilde(path)
    } else {
        expand_tilde(home_dir.join(path))
    }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use ipc_api::subnet_id::SubnetID;

    use super::expand_tilde;
    use super::Settings;

    fn parse_config(run_mode: &str) -> Settings {
        let current_dir = PathBuf::from(".");
        let default_dir = PathBuf::from("../config");
        Settings::new(&default_dir, &current_dir, run_mode).unwrap()
    }

    #[test]
    fn parse_default_config() {
        let settings = parse_config("");
        assert!(!settings.resolver.enabled());
    }

    #[test]
    fn parse_test_config() {
        let settings = parse_config("test");
        assert!(settings.resolver.enabled());
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = std::env::var("HOME").expect("should work on Linux");
        let home_project = PathBuf::from(format!("{}/.project", home));
        assert_eq!(expand_tilde("~/.project"), home_project);
        assert_eq!(expand_tilde("/foo/bar"), PathBuf::from("/foo/bar"));
        assert_eq!(expand_tilde("~foo/bar"), PathBuf::from("~foo/bar"));
    }

    #[test]
    fn parse_subnet_id() {
        // NOTE: It would not work with `t` prefix addresses unless the current network is changed.
        let id = "/r31415926/f2xwzbdu7z5sam6hc57xxwkctciuaz7oe5omipwbq";
        SubnetID::from_str(id).unwrap();
    }

    #[test]
    #[ignore = "https://github.com/consensus-shipyard/ipc/issues/303"]
    fn parse_empty_subnet_id() {
        assert!(SubnetID::from_str("").is_err())
    }
}

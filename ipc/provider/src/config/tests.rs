// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
use std::io::Write;
use std::str::FromStr;
use std::sync::{Arc, Condvar, Mutex};

use fvm_shared::address::Address;
use indoc::formatdoc;
use ipc_sdk::subnet_id::SubnetID;
use primitives::EthAddress;
use tempfile::NamedTempFile;
use url::Url;

use crate::config::{Config, ReloadableConfig};

// Arguments for the config's fields
const REPO_PATH: &str = "~/.ipc";
const CHILD_ID: &str = "/r123/f0100";
const CHILD_AUTH_TOKEN: &str = "CHILD_AUTH_TOKEN";
const PROVIDER_HTTP: &str = "http://127.0.0.1:3030/rpc/v1";
const ETH_ADDRESS: &str = "0x6be1ccf648c74800380d0520d797a170c808b624";

#[tokio::test]
async fn reload_works() {
    let config_str = config_str();

    let mut file = NamedTempFile::new().unwrap();
    let path = file
        .path()
        .as_os_str()
        .to_os_string()
        .into_string()
        .unwrap();

    file.write_all(config_str.as_bytes()).unwrap();

    let h = Arc::new(ReloadableConfig::new(path.clone()).unwrap());
    let original_config = h.get_config();

    // A simple barrier implementation for testing.
    // Refer to: https://web.mit.edu/rust-lang_v1.25/arch/amd64_ubuntu1404/share/doc/rust/html/std/sync/struct.Condvar.html#examples
    // Only when the main thread has created a new subscriber then we trigger then update the config file.
    // This way, we dont miss the update and stuck the main thread.
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair2 = pair.clone();
    let h_cloned = h.clone();
    tokio::spawn(async move {
        {
            let (lock, cvar) = &*pair;
            let mut started = lock.lock().unwrap();
            while !*started {
                started = cvar.wait(started).unwrap();
            }
        };

        let config_str = config_str_diff_addr();

        let mut file = file.reopen().unwrap();
        file.write_all(config_str.as_bytes()).unwrap();

        h_cloned.set_path(path);
        h_cloned.reload().await.unwrap();
    });

    let mut rx = h.new_subscriber();
    {
        let (lock, cvar) = &*pair2;
        let mut started = lock.lock().unwrap();
        *started = true;
        cvar.notify_one();
    }
    rx.recv().await.unwrap();

    let updated_config = h.get_config();
    assert_ne!(updated_config.keystore_path, original_config.keystore_path,);
}

#[test]
fn check_keystore_config() {
    let config = read_config();
    assert_eq!(
        config.keystore_path,
        Some(REPO_PATH.to_string()),
        "invalid provider keystore path"
    );
}

#[test]
fn check_subnets_config() {
    let config = read_config().subnets;

    let child_id = SubnetID::from_str(CHILD_ID).unwrap();
    let child = &config[&child_id];
    assert_eq!(child.id, child_id);
    assert_eq!(
        child.gateway_addr(),
        Address::from(EthAddress::from_str(ETH_ADDRESS).unwrap())
    );
    assert_eq!(*child.rpc_http(), Url::from_str(PROVIDER_HTTP).unwrap(),);
    assert_eq!(child.auth_token().as_ref().unwrap(), CHILD_AUTH_TOKEN);
}

fn config_str() -> String {
    formatdoc!(
        r#"
        keystore_path = "{REPO_PATH}"

        [[subnets]]
        id = "{CHILD_ID}"

        [subnets.config]
        network_type = "fevm"
        auth_token = "{CHILD_AUTH_TOKEN}"
        provider_http = "{PROVIDER_HTTP}"
        registry_addr = "{ETH_ADDRESS}"
        gateway_addr = "{ETH_ADDRESS}"
        "#
    )
}

fn config_str_diff_addr() -> String {
    formatdoc!(
        r#"
        repo_path = "~/.ipc2"

        [[subnets]]
        id = "{CHILD_ID}"

        [subnets.config]
        network_type = "fevm"
        auth_token = "{CHILD_AUTH_TOKEN}"
        provider_http = "{PROVIDER_HTTP}"
        registry_addr = "{ETH_ADDRESS}"
        gateway_addr = "{ETH_ADDRESS}"
        "#
    )
}

fn read_config() -> Config {
    Config::from_toml_str(config_str().as_str()).unwrap()
}

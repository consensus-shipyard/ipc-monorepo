// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
use fvm_shared::address::Address;
use ipc_agent::cli::{
    CommandLineHandler, CreateSubnet, CreateSubnetArgs, GlobalArguments, JoinSubnet,
    JoinSubnetArgs, KillSubnet, KillSubnetArgs, LeaveSubnet, LeaveSubnetArgs,
};
use ipc_api::subnet_id::SubnetID;
use std::str::FromStr;

pub struct TestClient {
    json_rpc_url: Option<String>,
}

impl TestClient {
    pub fn new(json_rpc_url: Option<String>) -> Self {
        Self { json_rpc_url }
    }

    pub async fn create_subnet(&self, parent: &str) -> anyhow::Result<Address> {
        let global = GlobalArguments::default();
        let args = CreateSubnetArgs {
            ipc_agent_url: self.json_rpc_url.clone(),
            from: None,
            parent: String::from(parent),
            name: "test".to_string(),
            min_validator_stake: 1.0,
            min_validators: 0,
            bottomup_check_period: 10,
            topdown_check_period: 10,
        };

        let raw = CreateSubnet::create(&global, &args).await?;
        Ok(Address::from_str(&raw)?)
    }

    pub async fn join_subnet(
        &self,
        subnet_id: &SubnetID,
        validator_net_addr: String,
    ) -> anyhow::Result<()> {
        JoinSubnet::handle(
            &GlobalArguments::default(),
            &JoinSubnetArgs {
                ipc_agent_url: self.json_rpc_url.clone(),
                from: None,
                subnet: subnet_id.to_string(),
                collateral: 10.0,
                validator_net_addr,
                worker_addr: None,
            },
        )
        .await
    }

    pub async fn kill_subnet(&self, subnet_id: &SubnetID) -> anyhow::Result<()> {
        KillSubnet::handle(
            &GlobalArguments::default(),
            &KillSubnetArgs {
                ipc_agent_url: self.json_rpc_url.clone(),
                from: None,
                subnet: subnet_id.to_string(),
            },
        )
        .await
    }

    pub async fn leave_subnet(&self, subnet_id: &SubnetID) -> anyhow::Result<()> {
        LeaveSubnet::handle(
            &GlobalArguments::default(),
            &LeaveSubnetArgs {
                ipc_agent_url: self.json_rpc_url.clone(),
                from: None,
                subnet: subnet_id.to_string(),
            },
        )
        .await
    }
}

// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

use std::fmt::Debug;
use std::str::FromStr;

use async_trait::async_trait;
use clap::Args;
use fvm_shared::address::Address;
use ipc_sdk::subnet_id::SubnetID;

use crate::commands::get_ipc_provider;
use crate::{CommandLineHandler, GlobalArguments};

/// The command to check if the address has submitted in the last bottom up checkpoint height.
pub(crate) struct SubmittedInBottomUpHeight;

#[async_trait]
impl CommandLineHandler for SubmittedInBottomUpHeight {
    type Arguments = SubmittedInBottomUpHeightArgs;

    async fn handle(global: &GlobalArguments, arguments: &Self::Arguments) -> anyhow::Result<()> {
        log::debug!(
            "check submitted bottom up checkpoint with args: {:?}",
            arguments
        );

        let provider = get_ipc_provider(global)?;
        let subnet = SubnetID::from_str(&arguments.subnet)?;
        let address = Address::from_str(&arguments.submitter)?;

        let submitted = provider
            .has_submitted_in_last_checkpoint_height(&subnet, &address)
            .await?;
        println!("has submitted: {submitted}");

        Ok(())
    }
}

#[derive(Debug, Args)]
#[command(
    about = "Check if the address has submitted a signature in the last bottom up checkpoint of a child subnet"
)]
pub(crate) struct SubmittedInBottomUpHeightArgs {
    #[arg(long, short, help = "The target subnet to perform query")]
    pub subnet: String,
    #[arg(long, short, help = "The hex encoded address of the submitter")]
    pub submitter: String,
}

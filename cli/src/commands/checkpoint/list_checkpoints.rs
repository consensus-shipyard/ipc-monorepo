// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
//! List checkpoints cli command

use std::fmt::Debug;

use async_trait::async_trait;
use clap::Args;
use fvm_shared::clock::ChainEpoch;

use crate::{CommandLineHandler, GlobalArguments};

/// The command to list checkpoints committed in a subnet actor.
pub(crate) struct ListBottomUpCheckpoints;

#[async_trait]
impl CommandLineHandler for ListBottomUpCheckpoints {
    type Arguments = ListBottomUpCheckpointsArgs;

    async fn handle(_global: &GlobalArguments, arguments: &Self::Arguments) -> anyhow::Result<()> {
        log::debug!("list checkpoints with args: {:?}", arguments);
        todo!()
    }
}

#[derive(Debug, Args)]
#[command(about = "List bottom-up checkpoints")]
pub(crate) struct ListBottomUpCheckpointsArgs {
    #[arg(long, short, help = "The subnet id of the checkpointing subnet")]
    pub subnet: String,
    #[arg(long, short, help = "Include checkpoints from this epoch")]
    pub from_epoch: ChainEpoch,
    #[arg(long, short, help = "Include checkpoints up to this epoch")]
    pub to_epoch: ChainEpoch,
}

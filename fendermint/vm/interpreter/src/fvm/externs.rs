// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
use cid::Cid;
use fvm::externs::{Chain, Consensus, Externs, Rand};
use fvm_shared::clock::ChainEpoch;

/// Dummy externs - these are related to Expected Consensus,
/// which I believe we have nothing to do with.
pub struct FendermintExterns;

impl Rand for FendermintExterns {
    fn get_chain_randomness(
        &self,
        _pers: i64,
        _round: ChainEpoch,
        _entropy: &[u8],
    ) -> anyhow::Result<[u8; 32]> {
        todo!("might need randomness")
    }

    fn get_beacon_randomness(
        &self,
        _pers: i64,
        _round: ChainEpoch,
        _entropy: &[u8],
    ) -> anyhow::Result<[u8; 32]> {
        unimplemented!("not expecting to use the beacon")
    }
}

impl Consensus for FendermintExterns {
    fn verify_consensus_fault(
        &self,
        _h1: &[u8],
        _h2: &[u8],
        _extra: &[u8],
    ) -> anyhow::Result<(Option<fvm_shared::consensus::ConsensusFault>, i64)> {
        unimplemented!("not expecting to use consensus faults")
    }
}

impl Chain for FendermintExterns {
    fn get_tipset_cid(&self, _epoch: ChainEpoch) -> anyhow::Result<Cid> {
        unimplemented!("not expecting to use tipsets")
    }
}

impl Externs for FendermintExterns {}

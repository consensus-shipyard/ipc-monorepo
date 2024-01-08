// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
use serde::{Deserialize, Serialize};

/// Unix timestamp (in seconds).
#[derive(Clone, Debug, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn as_secs(&self) -> i64 {
        self.0 as i64
    }
}

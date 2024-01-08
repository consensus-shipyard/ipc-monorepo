// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::{BlockHash, BlockHeight};
use ethers::utils::hex;
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug)]
pub(crate) struct SyncPointers {
    tail: Option<(BlockHeight, BlockHash)>,
    head: BlockHeight,
}

impl SyncPointers {
    pub fn new(head: BlockHeight) -> Self {
        Self { tail: None, head }
    }

    pub fn head(&self) -> BlockHeight {
        self.head
    }

    pub fn tail(&self) -> Option<(BlockHeight, BlockHash)> {
        self.tail.clone()
    }

    pub fn advance_head(&mut self) {
        self.head += 1;
    }

    pub fn set_tail(&mut self, height: BlockHeight, hash: BlockHash) {
        self.tail = Some((height, hash));
    }
}

impl Display for SyncPointers {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if let Some((height, hash)) = &self.tail {
            write!(
                f,
                "{{tail: {{height: {}, hash: {}}}, head: {}}}",
                height,
                hex::encode(hash),
                self.head
            )
        } else {
            write!(f, "{{tail: None, head: {}}}", self.head)
        }
    }
}

// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
//! Cross network messages related struct and utility functions.

use crate::address::IPCAddress;
use crate::subnet_id::SubnetID;
use anyhow::anyhow;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;
use num_traits::Zero;
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

/// StorableMsg stores all the relevant information required
/// to execute cross-messages.
///
/// We follow this approach because we can't directly store types.Message
/// as we did in the actor's Go counter-part. Instead we just persist the
/// information required to create the cross-messages and execute in the
/// corresponding node implementation.
#[derive(PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct StorableMsg {
    pub from: IPCAddress,
    pub to: IPCAddress,
    pub method: MethodNum,
    pub params: RawBytes,
    pub value: TokenAmount,
    pub nonce: u64,
    pub fee: TokenAmount,
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct CrossMsg {
    pub msg: StorableMsg,
    pub wrapped: bool,
}

#[derive(PartialEq, Eq)]
pub enum IPCMsgType {
    BottomUp,
    TopDown,
}

impl StorableMsg {
    pub fn new_release_msg(
        sub_id: &SubnetID,
        from: &Address,
        to: &Address,
        value: TokenAmount,
        fee: TokenAmount,
    ) -> anyhow::Result<Self> {
        let to = IPCAddress::new(
            &match sub_id.parent() {
                Some(s) => s,
                None => return Err(anyhow!("error getting parent for subnet addr")),
            },
            to,
        )?;
        let from = IPCAddress::new(sub_id, from)?;
        Ok(Self {
            from,
            to,
            method: METHOD_SEND,
            params: RawBytes::default(),
            value,
            nonce: 0,
            fee,
        })
    }

    pub fn new_fund_msg(
        sub_id: &SubnetID,
        from: &Address,
        to: &Address,
        value: TokenAmount,
    ) -> anyhow::Result<Self> {
        let from = IPCAddress::new(
            &match sub_id.parent() {
                Some(s) => s,
                None => return Err(anyhow!("error getting parent for subnet addr")),
            },
            from,
        )?;
        let to = IPCAddress::new(sub_id, to)?;
        // the nonce and the rest of message fields are set when the message is committed.
        Ok(Self {
            from,
            to,
            method: METHOD_SEND,
            params: RawBytes::default(),
            value,
            nonce: 0,
            fee: TokenAmount::zero(), // fund messages are currently free
        })
    }

    pub fn ipc_type(&self) -> anyhow::Result<IPCMsgType> {
        let sto = self.to.subnet()?;
        let sfrom = self.from.subnet()?;
        if is_bottomup(&sfrom, &sto) {
            return Ok(IPCMsgType::BottomUp);
        }
        Ok(IPCMsgType::TopDown)
    }

    pub fn apply_type(&self, curr: &SubnetID) -> anyhow::Result<IPCMsgType> {
        let sto = self.to.subnet()?;
        let sfrom = self.from.subnet()?;
        if curr.common_parent(&sto) == sfrom.common_parent(&sto)
            && self.ipc_type()? == IPCMsgType::BottomUp
        {
            return Ok(IPCMsgType::BottomUp);
        }
        Ok(IPCMsgType::TopDown)
    }
}

pub fn is_bottomup(from: &SubnetID, to: &SubnetID) -> bool {
    let index = match from.common_parent(to) {
        Some((ind, _)) => ind,
        None => return false,
    };
    // more children than the common parent
    from.children_as_ref().len() > index
}

#[derive(PartialEq, Eq, Clone, Debug, Default, Serialize_tuple, Deserialize_tuple)]
pub struct CrossMsgs {
    // FIXME: Consider to make this an AMT if we expect
    // a lot of cross-messages to be propagated.
    pub msgs: Vec<CrossMsg>,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone)]
struct ApplyMsgParams {
    pub cross_msg: CrossMsg,
}

impl CrossMsgs {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "fil-actor")]
impl CrossMsg {
    pub fn send(
        self,
        rt: &impl fil_actors_runtime::runtime::Runtime,
        rto: &Address,
    ) -> Result<RawBytes, fil_actors_runtime::ActorError> {
        let blk = if !self.wrapped {
            let msg = self.msg;
            rt.send(rto, msg.method, msg.params.into(), msg.value)?
        } else {
            let method = self.msg.method;
            let value = self.msg.value.clone();
            let params =
                fvm_ipld_encoding::ipld_block::IpldBlock::serialize_cbor(&ApplyMsgParams {
                    cross_msg: self,
                })?;
            rt.send(rto, method, params, value)?
        };

        Ok(match blk {
            Some(b) => b.data.into(), // FIXME: this assumes cbor serialization. We should maybe return serialized IpldBlock
            None => RawBytes::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::cross::*;
    use std::str::FromStr;

    #[test]
    fn test_is_bottomup() {
        bottom_up("/r123/f01", "/r123/f01/f02", false);
        bottom_up("/r123/f01", "/r123", true);
        bottom_up("/r123/f01", "/r123/f01/f02", false);
        bottom_up("/r123/f01", "/r123/f02/f02", true);
        bottom_up("/r123/f01/f02", "/r123/f01/f02", false);
        bottom_up("/r123/f01/f02", "/r123/f01/f02/f03", false);
    }

    fn bottom_up(a: &str, b: &str, res: bool) {
        assert_eq!(
            is_bottomup(
                &SubnetID::from_str(a).unwrap(),
                &SubnetID::from_str(b).unwrap()
            ),
            res
        );
    }
}

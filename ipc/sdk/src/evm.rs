// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

//! Type conversion for IPC Agent struct with solidity contract struct

use crate::address::IPCAddress;
use crate::checkpoint::BottomUpCheckpoint;
use crate::cross::{CrossMsg, StorableMsg};
use crate::staking::StakingChange;
use crate::staking::StakingChangeRequest;
use crate::subnet_id::SubnetID;
use crate::{eth_to_fil_amount, ethers_address_to_fil_address};
use anyhow::anyhow;
use ethers::types::U256;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::{Address, Payload};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::MethodNum;
use ipc_actors_abis::{
    gateway_getter_facet, gateway_manager_facet, gateway_messenger_facet, gateway_router_facet,
    lib_gateway, subnet_actor_getter_facet, subnet_actor_manager_facet,
};

/// The type conversion for IPC structs to evm solidity contracts. We need this convenient macro because
/// the abigen is creating the same struct but under different modules. This save a lot of
/// code.
macro_rules! base_type_conversion {
    ($module:ident) => {
        impl TryFrom<&SubnetID> for $module::SubnetID {
            type Error = anyhow::Error;

            fn try_from(subnet: &SubnetID) -> Result<Self, Self::Error> {
                Ok($module::SubnetID {
                    root: subnet.root_id(),
                    route: subnet_id_to_evm_addresses(subnet)?,
                })
            }
        }

        impl TryFrom<$module::SubnetID> for SubnetID {
            type Error = anyhow::Error;

            fn try_from(value: $module::SubnetID) -> Result<Self, Self::Error> {
                let children = value
                    .route
                    .iter()
                    .map(ethers_address_to_fil_address)
                    .collect::<anyhow::Result<Vec<_>>>()?;
                Ok(SubnetID::new(value.root, children))
            }
        }
    };
}

/// Implement the cross network message types. To use this macro, make sure the $module has already
/// implemented the base types.
macro_rules! cross_msg_types {
    ($module:ident) => {
        impl TryFrom<IPCAddress> for $module::Ipcaddress {
            type Error = anyhow::Error;

            fn try_from(value: IPCAddress) -> Result<Self, Self::Error> {
                Ok($module::Ipcaddress {
                    subnet_id: $module::SubnetID::try_from(&value.subnet()?)?,
                    raw_address: $module::FvmAddress::try_from(value.raw_addr()?)?,
                })
            }
        }

        impl TryFrom<$module::Ipcaddress> for IPCAddress {
            type Error = anyhow::Error;

            fn try_from(value: $module::Ipcaddress) -> Result<Self, Self::Error> {
                let addr = Address::try_from(value.raw_address)?;
                let i = IPCAddress::new(&SubnetID::try_from(value.subnet_id)?, &addr)?;
                Ok(i)
            }
        }

        impl TryFrom<StorableMsg> for $module::StorableMsg {
            type Error = anyhow::Error;

            fn try_from(value: StorableMsg) -> Result<Self, Self::Error> {
                let msg_value = fil_to_eth_amount(&value.value)?;
                let msg_fee = fil_to_eth_amount(&value.fee)?;

                log::info!(
                    "storable message token amount: {:}, converted: {:?}",
                    value.value.atto().to_string(),
                    msg_value
                );

                let c = $module::StorableMsg {
                    from: $module::Ipcaddress::try_from(value.from).map_err(|e| {
                        anyhow!("cannot convert `from` ipc address msg due to: {e:}")
                    })?,
                    to: $module::Ipcaddress::try_from(value.to)
                        .map_err(|e| anyhow!("cannot convert `to`` ipc address due to: {e:}"))?,
                    value: msg_value,
                    nonce: value.nonce,
                    // FIXME: we might a better way to handle the encoding of methods and params according to the type of message the cross-net message is targetting.
                    method: (value.method as u32).to_be_bytes(),
                    params: ethers::core::types::Bytes::from(value.params.to_vec()),
                    fee: msg_fee,
                };
                Ok(c)
            }
        }

        impl TryFrom<$module::StorableMsg> for StorableMsg {
            type Error = anyhow::Error;

            fn try_from(value: $module::StorableMsg) -> Result<Self, Self::Error> {
                let s = StorableMsg {
                    from: IPCAddress::try_from(value.from)?,
                    to: IPCAddress::try_from(value.to)?,
                    method: u32::from_be_bytes(value.method) as MethodNum,
                    params: RawBytes::from(value.params.to_vec()),
                    value: eth_to_fil_amount(&value.value)?,
                    nonce: value.nonce,
                    fee: eth_to_fil_amount(&value.fee)?,
                };
                Ok(s)
            }
        }

        impl TryFrom<CrossMsg> for $module::CrossMsg {
            type Error = anyhow::Error;

            fn try_from(value: CrossMsg) -> Result<Self, Self::Error> {
                let c = $module::CrossMsg {
                    wrapped: value.wrapped,
                    message: $module::StorableMsg::try_from(value.msg)
                        .map_err(|e| anyhow!("cannot convert storable msg due to: {e:}"))?,
                };
                Ok(c)
            }
        }

        impl TryFrom<$module::CrossMsg> for CrossMsg {
            type Error = anyhow::Error;

            fn try_from(value: $module::CrossMsg) -> Result<Self, Self::Error> {
                let c = CrossMsg {
                    wrapped: value.wrapped,
                    msg: StorableMsg::try_from(value.message)?,
                };
                Ok(c)
            }
        }
    };
}

/// The type conversion between different bottom up checkpoint definition in ethers and sdk
macro_rules! bottom_up_type_conversion {
    ($module:ident) => {
        impl TryFrom<BottomUpCheckpoint> for $module::BottomUpCheckpoint {
            type Error = anyhow::Error;

            fn try_from(checkpoint: BottomUpCheckpoint) -> Result<Self, Self::Error> {
                Ok($module::BottomUpCheckpoint {
                    subnet_id: $module::SubnetID::try_from(&checkpoint.subnet_id)?,
                    block_height: checkpoint.block_height as u64,
                    block_hash: vec_to_bytes32(checkpoint.block_hash)?,
                    next_configuration_number: checkpoint.next_configuration_number,
                    cross_messages_hash: vec_to_bytes32(checkpoint.cross_messages_hash)?,
                })
            }
        }

        impl TryFrom<$module::BottomUpCheckpoint> for BottomUpCheckpoint {
            type Error = anyhow::Error;

            fn try_from(value: $module::BottomUpCheckpoint) -> Result<Self, Self::Error> {
                Ok(BottomUpCheckpoint {
                    subnet_id: SubnetID::try_from(value.subnet_id)?,
                    block_height: value.block_height as ChainEpoch,
                    block_hash: value.block_hash.to_vec(),
                    next_configuration_number: value.next_configuration_number,
                    cross_messages_hash: value.cross_messages_hash.to_vec(),
                })
            }
        }
    };
}

base_type_conversion!(gateway_router_facet);
base_type_conversion!(subnet_actor_getter_facet);
base_type_conversion!(gateway_manager_facet);
base_type_conversion!(subnet_actor_manager_facet);
base_type_conversion!(gateway_getter_facet);
base_type_conversion!(gateway_messenger_facet);
base_type_conversion!(lib_gateway);

cross_msg_types!(gateway_getter_facet);
cross_msg_types!(gateway_router_facet);
cross_msg_types!(gateway_messenger_facet);
cross_msg_types!(subnet_actor_manager_facet);
cross_msg_types!(lib_gateway);

bottom_up_type_conversion!(gateway_getter_facet);
bottom_up_type_conversion!(subnet_actor_manager_facet);

/// Convert the ipc SubnetID type to a vec of evm addresses. It extracts all the children addresses
/// in the subnet id and turns them as a vec of evm addresses.
pub fn subnet_id_to_evm_addresses(
    subnet: &SubnetID,
) -> anyhow::Result<Vec<ethers::types::Address>> {
    let children = subnet.children();
    children
        .iter()
        .map(|addr| payload_to_evm_address(addr.payload()))
        .collect::<anyhow::Result<_>>()
}

/// Util function to convert Fil address payload to evm address. Only delegated address is supported.
pub fn payload_to_evm_address(payload: &Payload) -> anyhow::Result<ethers::types::Address> {
    match payload {
        Payload::Delegated(delegated) => {
            let slice = delegated.subaddress();
            Ok(ethers::types::Address::from_slice(&slice[0..20]))
        }
        _ => Err(anyhow!("address provided is not delegated")),
    }
}

/// Converts a Fil TokenAmount into an ethers::U256 amount.
pub fn fil_to_eth_amount(amount: &TokenAmount) -> anyhow::Result<U256> {
    let str = amount.atto().to_string();
    Ok(U256::from_dec_str(&str)?)
}

impl TryFrom<StakingChange> for gateway_router_facet::StakingChange {
    type Error = anyhow::Error;

    fn try_from(value: StakingChange) -> Result<Self, Self::Error> {
        Ok(gateway_router_facet::StakingChange {
            op: value.op as u8,
            payload: ethers::core::types::Bytes::from(value.payload),
            validator: payload_to_evm_address(value.validator.payload())?,
        })
    }
}

impl TryFrom<StakingChangeRequest> for gateway_router_facet::StakingChangeRequest {
    type Error = anyhow::Error;

    fn try_from(value: StakingChangeRequest) -> Result<Self, Self::Error> {
        Ok(gateway_router_facet::StakingChangeRequest {
            change: gateway_router_facet::StakingChange::try_from(value.change)?,
            configuration_number: value.configuration_number,
        })
    }
}

pub fn vec_to_bytes32(v: Vec<u8>) -> anyhow::Result<[u8; 32]> {
    if v.len() != 32 {
        return Err(anyhow!("invalid length"));
    }

    let mut r = [0u8; 32];
    r.copy_from_slice(&v);

    Ok(r)
}

#[cfg(test)]
mod tests {
    use crate::evm::subnet_id_to_evm_addresses;
    use crate::subnet_id::SubnetID;
    use fvm_shared::address::Address;
    use primitives::EthAddress;
    use std::str::FromStr;

    #[test]
    fn test_subnet_id_to_evm_addresses() {
        let eth_addr = EthAddress::from_str("0x0000000000000000000000000000000000000000").unwrap();
        let addr = Address::from(eth_addr);
        let addr2 = Address::from_str("f410ffzyuupbyl2uiucmzr3lu3mtf3luyknthaz4xsrq").unwrap();

        let id = SubnetID::new(0, vec![addr, addr2]);

        let addrs = subnet_id_to_evm_addresses(&id).unwrap();

        let a =
            ethers::types::Address::from_str("0x0000000000000000000000000000000000000000").unwrap();
        let b =
            ethers::types::Address::from_str("0x2e714a3c385ea88a09998ed74db265dae9853667").unwrap();

        assert_eq!(addrs, vec![a, b]);
    }
}

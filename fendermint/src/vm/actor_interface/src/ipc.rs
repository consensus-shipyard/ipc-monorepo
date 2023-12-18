// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

// The IPC actors have bindings in `ipc_actors_abis`.
// Here we define stable IDs for them, so we can deploy the
// Solidity contracts during genesis.

use anyhow::Context;
use ethers::core::abi::Tokenize;
use ethers::core::types as et;
use ethers::core::utils::keccak256;
use fendermint_vm_genesis::{Power, Validator};
use fvm_shared::address::Error as AddressError;
use fvm_shared::address::Payload;
use ipc_actors_abis as ia;
pub use ipc_actors_abis::gateway_router_facet::BottomUpCheckpoint;
use ipc_sdk::subnet_id::SubnetID;
use lazy_static::lazy_static;
use merkle_tree_rs::{
    core::{process_proof, Hash},
    format::Raw,
    standard::{standard_leaf_hash, LeafType, StandardMerkleTree},
};

use crate::{
    diamond::{EthContract, EthContractMap, EthFacet},
    eam::{EthAddress, EAM_ACTOR_ID},
};

define_id!(GATEWAY { id: 64 });
define_id!(SUBNETREGISTRY { id: 65 });

lazy_static! {
    /// Contracts deployed at genesis with well-known IDs.
    pub static ref IPC_CONTRACTS: EthContractMap = {
        [
            (
                gateway::CONTRACT_NAME,
                EthContract {
                    actor_id: GATEWAY_ACTOR_ID,
                    abi: ia::gateway_diamond::GATEWAYDIAMOND_ABI.to_owned(),
                    facets: vec![
                        EthFacet {
                            name: "GatewayGetterFacet",
                            abi: ia::gateway_getter_facet::GATEWAYGETTERFACET_ABI.to_owned(),
                        },
                        EthFacet {
                            name: "GatewayManagerFacet",
                            abi: ia::gateway_manager_facet::GATEWAYMANAGERFACET_ABI.to_owned(),
                        },
                        EthFacet {
                            name: "GatewayRouterFacet",
                            abi: ia::gateway_router_facet::GATEWAYROUTERFACET_ABI.to_owned(),
                        },
                        EthFacet {
                            name: "GatewayMessengerFacet",
                            abi: ia::gateway_messenger_facet::GATEWAYMESSENGERFACET_ABI.to_owned(),
                        },
                    ],
                },
            ),
            (
                registry::CONTRACT_NAME,
                EthContract {
                    actor_id: SUBNETREGISTRY_ACTOR_ID,
                    abi: ia::subnet_registry_diamond::SUBNETREGISTRYDIAMOND_ABI.to_owned(),
                    facets: vec![
                        // The registry incorporates the SubnetActor facets, although these aren't expected differently in the constructor.
                        EthFacet {
                            name: "SubnetActorGetterFacet",
                            abi: ia::subnet_actor_getter_facet::SUBNETACTORGETTERFACET_ABI
                                .to_owned(),
                        },
                        EthFacet {
                            name: "SubnetActorManagerFacet",
                            abi: ia::subnet_actor_manager_facet::SUBNETACTORMANAGERFACET_ABI
                                .to_owned(),
                        },
                        // The registry has its own facets:
                        // https://github.com/consensus-shipyard/ipc-solidity-actors/blob/b01a2dffe367745f55111a65536a3f6fea9165f5/scripts/deploy-registry.template.ts#L58-L67
                        EthFacet {
                            name: "RegisterSubnetFacet",
                            abi: ia::register_subnet_facet::REGISTERSUBNETFACET_ABI
                                .to_owned(),
                        },
                        EthFacet {
                            name: "SubnetGetterFacet",
                            abi: ia::subnet_getter_facet::SUBNETGETTERFACET_ABI
                                .to_owned(),
                        },
                        EthFacet {
                            name: "DiamondLoupeFacet",
                            abi: ia::diamond_loupe_facet::DIAMONDLOUPEFACET_ABI
                                .to_owned(),
                        },
                        EthFacet {
                            name: "DiamondCutFacet",
                            abi: ia::diamond_cut_facet::DIAMONDCUTFACET_ABI
                                .to_owned(),
                        },
                    ],
                },
            ),
        ]
        .into_iter()
        .collect()
    };

    /// Contracts that need to be deployed afresh for each subnet.
    ///
    /// See [deploy-sa-diamond.ts](https://github.com/consensus-shipyard/ipc-solidity-actors/blob/dev/scripts/deploy-sa-diamond.ts)
    ///
    /// But it turns out that the [SubnetRegistry](https://github.com/consensus-shipyard/ipc-solidity-actors/blob/3b0f3528b79e53e3c90f15016a40892122938ef0/src/SubnetRegistry.sol#L67)
    /// actor has this `SubnetActorDiamond` and its facets baked into it, and able to deploy without further ado.
    pub static ref SUBNET_CONTRACTS: EthContractMap = {
        [
            (
                subnet::CONTRACT_NAME,
                EthContract {
                    actor_id: 0,
                    abi: ia::subnet_actor_diamond::SUBNETACTORDIAMOND_ABI.to_owned(),
                    facets: vec![
                        EthFacet {
                            name: "SubnetActorGetterFacet",
                            abi: ia::subnet_actor_getter_facet::SUBNETACTORGETTERFACET_ABI.to_owned(),
                        },
                        EthFacet {
                            name: "SubnetActorManagerFacet",
                            abi: ia::subnet_actor_manager_facet::SUBNETACTORMANAGERFACET_ABI.to_owned(),
                        },
                    ],
                },
            ),
        ]
        .into_iter()
        .collect()
    };

    /// ABI types of the Merkle tree which contains validator addresses and their voting power.
    pub static ref VALIDATOR_TREE_FIELDS: Vec<String> =
        vec!["address".to_owned(), "uint256".to_owned()];
}

/// Construct a Merkle tree from the power table in a format which can be validated by
/// https://github.com/OpenZeppelin/openzeppelin-contracts/blob/master/contracts/utils/cryptography/MerkleProof.sol
///
/// The reference implementation is https://github.com/OpenZeppelin/merkle-tree/
pub struct ValidatorMerkleTree {
    tree: StandardMerkleTree<Raw>,
}

impl ValidatorMerkleTree {
    pub fn new(validators: &[Validator<Power>]) -> anyhow::Result<Self> {
        // Using the 20 byte address for keys because that's what the Solidity library returns
        // when recovering a public key from a signature.
        let values = validators
            .iter()
            .map(Self::validator_to_vec)
            .collect::<anyhow::Result<Vec<_>>>()?;

        let tree = StandardMerkleTree::of(&values, &VALIDATOR_TREE_FIELDS)
            .context("failed to construct Merkle tree")?;

        Ok(Self { tree })
    }

    pub fn root_hash(&self) -> Hash {
        self.tree.root()
    }

    /// Create a Merkle proof for a validator.
    pub fn prove(&self, validator: &Validator<Power>) -> anyhow::Result<Vec<Hash>> {
        let v = Self::validator_to_vec(validator)?;
        let proof = self
            .tree
            .get_proof(LeafType::LeafBytes(v))
            .context("failed to produce Merkle proof")?;
        Ok(proof)
    }

    /// Validate a proof against a known root hash.
    pub fn validate(
        validator: &Validator<Power>,
        root: &Hash,
        proof: &[Hash],
    ) -> anyhow::Result<bool> {
        let v = Self::validator_to_vec(validator)?;
        let h = standard_leaf_hash(v, &VALIDATOR_TREE_FIELDS)?;
        let r = process_proof(&h, proof).context("failed to process Merkle proof")?;
        Ok(*root == r)
    }

    /// Convert a validator to what we can pass to the tree.
    fn validator_to_vec(validator: &Validator<Power>) -> anyhow::Result<Vec<String>> {
        let addr = EthAddress::from(validator.public_key.0);
        let addr = et::Address::from_slice(&addr.0);
        let addr = format!("{addr:?}");

        let power = et::U256::from(validator.power.0);
        let power = power.to_string();
        Ok(vec![addr, power])
    }
}

/// Decompose a subnet ID into a root ID and a route of Ethereum addresses
pub fn subnet_id_to_eth(subnet_id: &SubnetID) -> Result<(u64, Vec<et::Address>), AddressError> {
    // Every step along the way in the subnet ID we have an Ethereum address.
    let mut route = Vec::new();
    for addr in subnet_id.children() {
        let addr = match addr.payload() {
            Payload::ID(id) => EthAddress::from_id(*id),
            Payload::Delegated(da)
                if da.namespace() == EAM_ACTOR_ID && da.subaddress().len() == 20 =>
            {
                EthAddress(da.subaddress().try_into().expect("checked length"))
            }
            _ => return Err(AddressError::InvalidPayload),
        };
        route.push(et::H160::from(addr.0))
    }
    Ok((subnet_id.root_id(), route))
}

/// Hash some value in the same way we'd hash it in Solidity.
///
/// Be careful that if we have to hash a single struct,
/// Solidity's `abi.encode` function will treat it as a tuple,
/// so it has to be passed as a tuple in Rust. Vectors are fine.
pub fn abi_hash<T: Tokenize>(value: T) -> [u8; 32] {
    keccak256(ethers::abi::encode(&value.into_tokens()))
}

/// Types where we need to match the way we sign them in Solidity and Rust.
pub trait AbiHash {
    /// Hash the item the way we would in Solidity.
    fn abi_hash(self) -> [u8; 32];
}

macro_rules! abi_hash {
    (struct $name:ty) => {
        // Structs have to be hashed as a tuple.
        impl AbiHash for $name {
            fn abi_hash(self) -> [u8; 32] {
                abi_hash((self,))
            }
        }
    };

    (Vec < $name:ty >) => {
        // Vectors can be hashed as-is
        impl AbiHash for Vec<$name> {
            fn abi_hash(self) -> [u8; 32] {
                abi_hash(self)
            }
        }
    };
}

abi_hash!(struct ipc_actors_abis::gateway_router_facet::BottomUpCheckpoint);
abi_hash!(struct ipc_actors_abis::subnet_actor_manager_facet::BottomUpCheckpoint);
abi_hash!(Vec<ipc_actors_abis::gateway_getter_facet::CrossMsg>);
abi_hash!(Vec<ipc_actors_abis::subnet_actor_manager_facet::CrossMsg>);
abi_hash!(Vec<ipc_actors_abis::subnet_actor_getter_facet::CrossMsg>);

pub mod gateway {
    use super::subnet_id_to_eth;
    use ethers::contract::{EthAbiCodec, EthAbiType};
    use ethers::core::types::{Bytes, H160, U256};
    use fendermint_vm_genesis::ipc::GatewayParams;
    use fendermint_vm_genesis::{Collateral, Validator};
    use fvm_shared::address::Error as AddressError;
    use fvm_shared::econ::TokenAmount;

    pub use ipc_actors_abis::gateway_getter_facet::Validator as GatewayValidator;
    use ipc_actors_abis::gateway_router_facet::SubnetID as GatewaySubnetID;

    use crate::eam::EthAddress;

    pub const CONTRACT_NAME: &str = "GatewayDiamond";
    pub const METHOD_INVOKE_CONTRACT: u64 = crate::evm::Method::InvokeContract as u64;

    // Constructor parameters aren't generated as part of the Rust bindings.
    // TODO: Remove these once https://github.com/gakonst/ethers-rs/pull/2631 is merged.

    /// Container type `ConstructorParameters`.
    ///
    /// See [GatewayDiamond.sol](https://github.com/consensus-shipyard/ipc-solidity-actors/blob/255da67fd6ad885f0ab633311be276a4fa936d45/src/GatewayDiamond.sol#L21)
    #[derive(Clone, EthAbiType, EthAbiCodec, Default, Debug, PartialEq, Eq, Hash)]
    pub struct ConstructorParameters {
        pub network_name: GatewaySubnetID,
        pub bottom_up_check_period: u64,
        pub min_collateral: U256,
        pub msg_fee: U256,
        pub majority_percentage: u8,
        pub validators: Vec<GatewayValidator>,
        pub active_validators_limit: u16,
    }

    impl ConstructorParameters {
        pub fn new(
            params: GatewayParams,
            validators: Vec<Validator<Collateral>>,
        ) -> Result<Self, AddressError> {
            // Every validator has an Ethereum address.
            let validators = validators
                .into_iter()
                .map(|v| {
                    let pk = v.public_key.0.serialize();
                    let addr = EthAddress::new_secp256k1(&pk)?;
                    let collateral = tokens_to_u256(v.power.0);
                    Ok(GatewayValidator {
                        addr: H160::from(addr.0),
                        weight: collateral,
                        metadata: Bytes::from(pk),
                    })
                })
                .collect::<Result<Vec<_>, AddressError>>()?;

            let (root, route) = subnet_id_to_eth(&params.subnet_id)?;

            Ok(Self {
                network_name: GatewaySubnetID { root, route },
                bottom_up_check_period: params.bottom_up_check_period,
                min_collateral: tokens_to_u256(params.min_collateral),
                msg_fee: tokens_to_u256(params.msg_fee),
                majority_percentage: params.majority_percentage,
                validators,
                active_validators_limit: params.active_validators_limit,
            })
        }
    }

    fn tokens_to_u256(value: TokenAmount) -> U256 {
        // XXX: Ignoring any error resulting from larger fee than what fits into U256. This is in genesis after all.
        U256::from_big_endian(&value.atto().to_bytes_be().1)
    }

    #[cfg(test)]
    mod tests {
        use ethers::core::types::{Selector, U256};
        use ethers_core::{
            abi::Tokenize,
            types::{Bytes, H160},
        };
        use fvm_shared::{bigint::BigInt, econ::TokenAmount};
        use ipc_actors_abis::gateway_getter_facet::Validator as GatewayValidator;
        use ipc_actors_abis::gateway_router_facet::SubnetID as GatewaySubnetID;
        use std::str::FromStr;

        use crate::ipc::tests::{check_param_types, constructor_param_types};

        use super::{tokens_to_u256, ConstructorParameters};

        #[test]
        fn tokenize_constructor_params() {
            let cp = ConstructorParameters {
                network_name: GatewaySubnetID {
                    root: 0,
                    route: Vec::new(),
                },
                bottom_up_check_period: 100,
                min_collateral: U256::from(1),
                msg_fee: U256::from(0),
                majority_percentage: 67,
                validators: vec![GatewayValidator {
                    addr: H160::zero(),
                    weight: U256::zero(),
                    metadata: Bytes::new(),
                }],
                active_validators_limit: 100,
            };

            // It looks like if we pass just the record then it will be passed as 5 tokens,
            // but the constructor only expects one parameter, and it has to be a tuple.
            let cp = (Vec::<Selector>::new(), cp);

            let tokens = cp.into_tokens();

            let cons = ipc_actors_abis::gateway_diamond::GATEWAYDIAMOND_ABI
                .constructor()
                .expect("Gateway has a constructor");

            let param_types = constructor_param_types(cons);

            check_param_types(&tokens, &param_types).unwrap();

            cons.encode_input(vec![], &tokens)
                .expect("should encode constructor input");
        }

        #[test]
        #[should_panic]
        fn max_fee_exceeded() {
            let mut value = BigInt::from_str(&U256::MAX.to_string()).unwrap();
            value += 1;
            let value = TokenAmount::from_atto(value);
            let _ = tokens_to_u256(value);
        }
    }
}

pub mod registry {
    use ethers::contract::{EthAbiCodec, EthAbiType};
    use ethers::core::types::Address;

    type FunctionSelector = [u8; 4];

    pub const CONTRACT_NAME: &str = "SubnetRegistryDiamond";

    /// Container type `ConstructorParameters`.
    ///
    /// See [SubnetRegistry.sol](https://github.com/consensus-shipyard/ipc-solidity-actors/blob/a830a52b1362f3d2abf2e3cc3db62aa40ee45355/src/SubnetRegistryDiamond.sol#L17-L23)
    #[derive(Clone, EthAbiType, EthAbiCodec, Default, Debug, PartialEq, Eq, Hash)]
    pub struct ConstructorParameters {
        pub gateway: Address,
        pub getter_facet: Address,
        pub manager_facet: Address,
        pub subnet_getter_selectors: Vec<FunctionSelector>,
        pub subnet_manager_selectors: Vec<FunctionSelector>,
    }
}

pub mod subnet {
    use crate::revert_errors;
    use ipc_actors_abis::gateway_manager_facet::GatewayManagerFacetErrors;
    use ipc_actors_abis::gateway_router_facet::GatewayRouterFacetErrors;
    use ipc_actors_abis::subnet_actor_manager_facet::SubnetActorManagerFacetErrors;

    pub const CONTRACT_NAME: &str = "SubnetActorDiamond";

    // The subnet actor has its own errors, but it also invokes the gateway, which might revert for its own reasons.
    revert_errors! {
        SubnetActorErrors {
            SubnetActorManagerFacetErrors,
            GatewayManagerFacetErrors,
            GatewayRouterFacetErrors
        }
    }

    #[cfg(test)]
    mod tests {
        use ethers::abi::{AbiType, Tokenize};
        use ethers::core::types::Bytes;
        use ipc_actors_abis::subnet_actor_manager_facet::{BottomUpCheckpoint, SubnetID};

        #[test]
        fn checkpoint_abi() {
            // Some random checkpoint printed in a test that failed because the Rust ABI was different then the Solidity ABI.
            let checkpoint = BottomUpCheckpoint {
                subnet_id: SubnetID {
                    root: 12378393254986206693,
                    route: vec![
                        "0x7b11cf9ca8ccee13bb3d003c97af5c18434067a9",
                        "0x3d9019b8bf3bfd5e979ddc3b2761be54af867c47",
                    ]
                    .into_iter()
                    .map(|h| h.parse().unwrap())
                    .collect(),
                },
                block_height: 21,
                block_hash: [
                    107, 115, 111, 52, 42, 179, 77, 154, 254, 66, 52, 169, 43, 219, 25, 12, 53,
                    178, 232, 216, 34, 217, 96, 27, 0, 185, 215, 8, 155, 25, 15, 1,
                ],
                next_configuration_number: 1,
                cross_messages_hash: [
                    86, 158, 117, 252, 119, 193, 168, 86, 246, 218, 175, 158, 105, 216, 169, 86,
                    108, 163, 74, 164, 127, 145, 51, 113, 28, 224, 101, 165, 113, 175, 12, 253,
                ],
            };

            let param_type = BottomUpCheckpoint::param_type();

            // Captured value of `abi.encode` in Solidity.
            let expected_abi: Bytes = "0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000156b736f342ab34d9afe4234a92bdb190c35b2e8d822d9601b00b9d7089b190f010000000000000000000000000000000000000000000000000000000000000001569e75fc77c1a856f6daaf9e69d8a9566ca34aa47f9133711ce065a571af0cfd000000000000000000000000000000000000000000000000abc8e314f58b4de5000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000020000000000000000000000007b11cf9ca8ccee13bb3d003c97af5c18434067a90000000000000000000000003d9019b8bf3bfd5e979ddc3b2761be54af867c47".parse().unwrap();

            // XXX: It doesn't work with `decode_whole`.
            let expected_tokens =
                ethers::abi::decode(&[param_type], &expected_abi).expect("invalid Solidity ABI");

            // The data needs to be wrapped into a tuple.
            let observed_tokens = (checkpoint,).into_tokens();
            let observed_abi: Bytes = ethers::abi::encode(&observed_tokens).into();

            assert_eq!(observed_tokens, expected_tokens);
            assert_eq!(observed_abi, expected_abi);
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::bail;
    use ethers_core::abi::{Constructor, ParamType, Token};
    use fendermint_vm_genesis::{Power, Validator};
    use quickcheck_macros::quickcheck;

    use super::ValidatorMerkleTree;

    /// Check all tokens against expected parameters; return any offending one.
    ///
    /// Based on [Tokens::types_check]
    pub fn check_param_types(tokens: &[Token], param_types: &[ParamType]) -> anyhow::Result<()> {
        if param_types.len() != tokens.len() {
            bail!(
                "different number of parameters; expected {}, got {}",
                param_types.len(),
                tokens.len()
            );
        }

        for (i, (pt, t)) in param_types.iter().zip(tokens).enumerate() {
            if !t.type_check(pt) {
                bail!("parameter {i} didn't type check: expected {pt:?}, got {t:?}");
            }
        }

        Ok(())
    }

    /// Returns all input params of given constructor.
    ///
    /// Based on [Constructor::param_types]
    pub fn constructor_param_types(cons: &Constructor) -> Vec<ParamType> {
        cons.inputs.iter().map(|p| p.kind.clone()).collect()
    }

    #[quickcheck]
    fn merkleize_validators(validators: Vec<Validator<Power>>) {
        if validators.is_empty() {
            return;
        }

        let tree = ValidatorMerkleTree::new(&validators).expect("failed to create tree");
        let root = tree.root_hash();

        let validator = validators.first().unwrap();
        let proof = tree.prove(validator).expect("failed to prove");

        assert!(ValidatorMerkleTree::validate(validator, &root, &proof).expect("failed to validate"))
    }
}

// Copyright 2022-2023 Protocol Labs
// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::anyhow;
use cid::multihash::MultihashDigest;
use cid::Cid;
use ethers_core::types as et;
use fendermint_crypto::SecretKey;
use fendermint_vm_actor_interface::{eam, evm};
use fvm_ipld_encoding::tuple::{Deserialize_tuple, Serialize_tuple};
use fvm_shared::address::{Address, Payload};
use fvm_shared::chainid::ChainID;
use fvm_shared::crypto::signature::{Signature, SignatureType, SECP_SIG_LEN};
use fvm_shared::message::Message;

use thiserror::Error;

use crate::conv::from_fvm;

enum Signable {
    /// Pair of transaction hash and from.
    Ethereum((et::H256, et::H160)),
    /// Bytes to be passed to the FVM Signature for hashing or verification.
    Regular(Vec<u8>),
}

#[derive(Error, Debug)]
pub enum SignedMessageError {
    #[error("message cannot be serialized")]
    Ipld(#[from] fvm_ipld_encoding::Error),
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    #[error("message cannot be converted to ethereum")]
    Ethereum(#[from] anyhow::Error),
}

/// Domain specific transaction hash.
///
/// Some tools like ethers.js refuse to accept Tendermint hashes,
/// which use a different algorithm than Ethereum.
///
/// We can potentially extend this list to include CID based indexing.
pub enum DomainHash {
    Eth([u8; 32]),
}

/// Represents a wrapped message with signature bytes.
///
/// This is the message that the client needs to send, but only the `message`
/// part is signed over.
///
/// Tuple serialization is used because it might result in a more compact data structure for storage,
/// and because the `Message` is already serialized as a tuple.
#[derive(PartialEq, Clone, Debug, Serialize_tuple, Deserialize_tuple, Hash, Eq)]
pub struct SignedMessage {
    pub message: Message,
    pub signature: Signature,
}

impl SignedMessage {
    /// Generate a new signed message from fields.
    ///
    /// The signature will not be verified.
    pub fn new_unchecked(message: Message, signature: Signature) -> SignedMessage {
        SignedMessage { message, signature }
    }

    /// Create a signed message.
    pub fn new_secp256k1(
        message: Message,
        sk: &SecretKey,
        chain_id: &ChainID,
    ) -> Result<Self, SignedMessageError> {
        let signature = match Self::signable(&message, chain_id)? {
            Signable::Ethereum((hash, _)) => sign_eth(sk, hash),
            Signable::Regular(data) => sign_regular(sk, &data),
        };
        Ok(Self { message, signature })
    }

    /// Calculate the CID of an FVM message.
    pub fn cid(message: &Message) -> Result<Cid, fvm_ipld_encoding::Error> {
        crate::cid(message)
    }

    /// Calculate the bytes that need to be signed.
    ///
    /// The [`ChainID`] is used as a replay attack protection, a variation of
    /// https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0039.md
    fn signable(message: &Message, chain_id: &ChainID) -> Result<Signable, SignedMessageError> {
        // Here we look at the sender to decide what scheme to use for hashing.
        //
        // This is in contrast to https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0055.md#delegated-signature-type
        // which introduces a `SignatureType::Delegated`, in which case the signature check should be done by the recipient actor.
        //
        // However, that isn't implemented, and adding that type would mean copying the entire `Signature` type into Fendermint,
        // similarly to how Forest did it https://github.com/ChainSafe/forest/blob/b3c5efe6cc81607da945227bb41c60cec47909c3/utils/forest_shim/src/crypto.rs#L166
        //
        // Instead of special casing on the signature type, we are special casing on the sender,
        // which should be okay because the CLI only uses `f1` addresses and the Ethereum API only uses `f410` addresses,
        // so at least for now they are easy to tell apart: any `f410` address is coming from Ethereum API and must have
        // been signed according to the Ethereum scheme, and it could not have been signed by an `f1` address, it doesn't
        // work with regular accounts.
        match maybe_eth_address(&message.from) {
            Some(addr) => {
                let tx = from_fvm::to_eth_typed_transaction(message, chain_id)
                    .map_err(SignedMessageError::Ethereum)?;

                Ok(Signable::Ethereum((tx.sighash(), addr)))
            }
            None => {
                let mut data = Self::cid(message)?.to_bytes();
                data.extend(chain_id_bytes(chain_id).iter());
                Ok(Signable::Regular(data))
            }
        }
    }

    /// Verify that the message CID was signed by the `from` address.
    pub fn verify_signature(
        message: &Message,
        signature: &Signature,
        chain_id: &ChainID,
    ) -> Result<(), SignedMessageError> {
        match Self::signable(message, chain_id)? {
            Signable::Ethereum((hash, from)) => {
                // If the sender is ethereum, recover the public key from the signature (which verifies it),
                // then turn it into an `EthAddress` and verify it matches the `from` of the message.
                let sig = from_fvm::to_eth_signature(signature, true)
                    .map_err(SignedMessageError::Ethereum)?;

                let rec = sig
                    .recover(hash)
                    .map_err(|e| SignedMessageError::Ethereum(anyhow!(e)))?;

                if rec == from {
                    verify_eth_method(message)
                } else {
                    Err(SignedMessageError::InvalidSignature(format!("the Ethereum delegated address did not match the one recovered from the signature (sighash = {:?})", hash)))
                }
            }
            Signable::Regular(data) => {
                // This works when `from` corresponds to the signature type.
                signature
                    .verify(&data, &message.from)
                    .map_err(SignedMessageError::InvalidSignature)
            }
        }
    }

    /// Calculate an optional hash that ecosystem tools expect.
    pub fn domain_hash(
        &self,
        chain_id: &ChainID,
    ) -> Result<Option<DomainHash>, SignedMessageError> {
        if maybe_eth_address(&self.message.from).is_some() {
            let tx = from_fvm::to_eth_typed_transaction(self.message(), chain_id)
                .map_err(SignedMessageError::Ethereum)?;

            let sig = from_fvm::to_eth_signature(self.signature(), true)
                .map_err(SignedMessageError::Ethereum)?;

            let rlp = tx.rlp_signed(&sig);

            let hash = cid::multihash::Code::Keccak256.digest(&rlp);
            let hash = hash.digest().try_into().expect("Keccak256 is 32 bytes");

            Ok(Some(DomainHash::Eth(hash)))
        } else {
            // Use the default transaction ID.
            Ok(None)
        }
    }

    /// Verifies that the from address of the message generated the signature.
    pub fn verify(&self, chain_id: &ChainID) -> Result<(), SignedMessageError> {
        Self::verify_signature(&self.message, &self.signature, chain_id)
    }

    /// Returns reference to the unsigned message.
    pub fn message(&self) -> &Message {
        &self.message
    }

    /// Returns signature of the signed message.
    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    /// Consumes self and returns it's unsigned message.
    pub fn into_message(self) -> Message {
        self.message
    }

    /// Checks if the signed message is a BLS message.
    pub fn is_bls(&self) -> bool {
        self.signature.signature_type() == SignatureType::BLS
    }

    /// Checks if the signed message is a SECP message.
    pub fn is_secp256k1(&self) -> bool {
        self.signature.signature_type() == SignatureType::Secp256k1
    }
}

/// Sign a transaction pre-image using Blake2b256, in a way that [Signature::verify] expects it.
fn sign_regular(sk: &SecretKey, data: &[u8]) -> Signature {
    let hash: [u8; 32] = blake2b_simd::Params::new()
        .hash_length(32)
        .to_state()
        .update(data)
        .finalize()
        .as_bytes()
        .try_into()
        .unwrap();

    sign_secp256k1(sk, &hash)
}

/// Sign a transaction pre-image in the same way Ethereum clients would sign it.
fn sign_eth(sk: &SecretKey, hash: et::H256) -> Signature {
    sign_secp256k1(sk, &hash.0)
}

/// Turn a [`ChainID`] into bytes. Uses big-endian encoding.
pub fn chain_id_bytes(chain_id: &ChainID) -> [u8; 8] {
    u64::from(*chain_id).to_be_bytes()
}

/// Return the 20 byte Ethereum address if the address is that kind of delegated one.
fn maybe_eth_address(addr: &Address) -> Option<et::H160> {
    match addr.payload() {
        Payload::Delegated(addr)
            if addr.namespace() == eam::EAM_ACTOR_ID && addr.subaddress().len() == 20 =>
        {
            Some(et::H160::from_slice(addr.subaddress()))
        }
        _ => None,
    }
}

/// Verify that the method ID and the recipient are one of the allowed combination,
/// which for example is set by [from_eth::to_fvm_message].
///
/// The method ID is not part of the signature, so someone could modify it, which is
/// why we have to check explicitly that there is nothing untowards going on.
fn verify_eth_method(msg: &Message) -> Result<(), SignedMessageError> {
    if msg.to == eam::EAM_ACTOR_ADDR {
        if msg.method_num != eam::Method::CreateExternal as u64 {
            return Err(SignedMessageError::Ethereum(anyhow!(
                "The EAM actor can only be called with CreateExternal; got {}",
                msg.method_num
            )));
        }
    } else if msg.method_num != evm::Method::InvokeContract as u64 {
        return Err(SignedMessageError::Ethereum(anyhow!(
            "An EVM actor can only be called with InvokeContract; got {} - {}",
            msg.to,
            msg.method_num
        )));
    }
    Ok(())
}

/// Sign a hash using the secret key.
pub fn sign_secp256k1(sk: &SecretKey, hash: &[u8; 32]) -> Signature {
    let (sig, recovery_id) = sk.sign(hash);

    let mut signature = [0u8; SECP_SIG_LEN];
    signature[..64].copy_from_slice(&sig.serialize());
    signature[64] = recovery_id.serialize();

    Signature {
        sig_type: SignatureType::Secp256k1,
        bytes: signature.to_vec(),
    }
}

/// Signed message with an invalid random signature.
#[cfg(feature = "arb")]
mod arb {
    use fendermint_testing::arb::ArbMessage;
    use fvm_shared::crypto::signature::Signature;

    use super::SignedMessage;

    /// An arbitrary `SignedMessage` that is at least as consistent as required for serialization.
    impl quickcheck::Arbitrary for SignedMessage {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            Self {
                message: ArbMessage::arbitrary(g).0,
                signature: Signature::arbitrary(g),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fendermint_vm_actor_interface::eam::EthAddress;
    use fvm_shared::{address::Address, chainid::ChainID};
    use quickcheck_macros::quickcheck;

    use crate::conv::tests::{EthMessage, KeyPair};

    use super::SignedMessage;

    #[quickcheck]
    fn chain_id_in_signature(
        msg: SignedMessage,
        chain_id: u64,
        key: KeyPair,
    ) -> Result<(), String> {
        let KeyPair { sk, pk } = key;

        let chain_id0 = ChainID::from(chain_id);
        let chain_id1 = ChainID::from(chain_id.overflowing_add(1).0);

        let mut msg = msg.into_message();
        msg.from = Address::new_secp256k1(&pk.serialize())
            .map_err(|e| format!("failed to conver to address: {e}"))?;

        let signed = SignedMessage::new_secp256k1(msg, &sk, &chain_id0)
            .map_err(|e| format!("signing failed: {e}"))?;

        signed
            .verify(&chain_id0)
            .map_err(|e| format!("verifying failed: {e}"))?;

        if signed.verify(&chain_id1).is_ok() {
            return Err("verifying with a different chain ID should fail".into());
        }
        Ok(())
    }

    #[quickcheck]
    fn eth_sign_and_verify(msg: EthMessage, chain_id: u64, key: KeyPair) -> Result<(), String> {
        let chain_id = ChainID::from(chain_id);
        let KeyPair { sk, pk } = key;

        // Set the message to the address we are going to sign with.
        let ea = EthAddress::from(pk);
        let mut msg = msg.0;
        msg.from = Address::from(ea);

        let signed =
            SignedMessage::new_secp256k1(msg, &sk, &chain_id).map_err(|e| e.to_string())?;

        signed.verify(&chain_id).map_err(|e| e.to_string())
    }
}

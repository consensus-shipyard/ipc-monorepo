// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT
use ipc_sdk::subnet_id::SubnetID;
use libipld::Cid;
use libp2p::identity::{Keypair, PublicKey};
use serde::de::Error;
use serde::{Deserialize, Serialize};

use crate::{
    signed_record::{Record, SignedRecord},
    Timestamp,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ValidatorKey(PublicKey);

impl Serialize for ValidatorKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let bz = self.0.to_protobuf_encoding();
        bz.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ValidatorKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bz = Vec::<u8>::deserialize(deserializer)?;
        match PublicKey::from_protobuf_encoding(&bz) {
            Ok(pk) => Ok(Self(pk)),
            Err(e) => Err(D::Error::custom(format!("error decoding PublicKey: {e}"))),
        }
    }
}

/// Vote by a validator about the validity/availability/finality
/// of a CID in a given subnet.
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct VoteRecord {
    /// Public key of the validator.
    pub public_key: ValidatorKey,
    /// The subnet in which the vote is valid, to prevent a vote on the same CID
    /// in one subnet being replayed by an attacker on a different subnet.
    pub subnet_id: SubnetID,
    /// The CID of the content the vote is about.
    pub cid: Cid,
    /// The claim of the vote, in case there can be votes about multiple facets
    /// regarding the CID.
    pub claim: String,
    /// Timestamp to thwart potential replay attacks.
    pub timestamp: Timestamp,
}

impl Record for VoteRecord {
    fn payload_type() -> &'static str {
        "/ipc/vote-record"
    }

    fn check_signing_key(&self, key: &libp2p::identity::PublicKey) -> bool {
        self.public_key.0 == *key
    }
}

pub type SignedVoteRecord = SignedRecord<VoteRecord>;

impl VoteRecord {
    /// Create a new [`SignedVoteRecord`] with the current timestamp
    /// and a signed envelope which can be shared with others.
    pub fn signed(
        key: &Keypair,
        subnet_id: SubnetID,
        cid: Cid,
        claim: String,
    ) -> anyhow::Result<SignedVoteRecord> {
        let timestamp = Timestamp::now();
        let record = VoteRecord {
            public_key: ValidatorKey(key.public()),
            subnet_id,
            cid,
            claim,
            timestamp,
        };
        let signed = SignedRecord::new(key, record)?;
        Ok(signed)
    }
}

#[cfg(any(test, feature = "arb"))]
mod arb {
    use libp2p::identity::Keypair;
    use quickcheck::Arbitrary;

    use crate::arb::{ArbCid, ArbSubnetID};

    use super::{SignedVoteRecord, VoteRecord};

    /// Create a valid [`SignedVoteRecord`] with a random key.
    impl Arbitrary for SignedVoteRecord {
        fn arbitrary(g: &mut quickcheck::Gen) -> Self {
            let key = Keypair::generate_secp256k1();
            let subnet_id = ArbSubnetID::arbitrary(g).0;
            let cid = ArbCid::arbitrary(g).0;
            let claim = String::arbitrary(g);

            VoteRecord::signed(&key, subnet_id, cid, claim).expect("error creating signed envelope")
        }
    }
}

#[cfg(test)]
mod tests {
    use quickcheck_macros::quickcheck;

    use super::SignedVoteRecord;

    #[quickcheck]
    fn prop_roundtrip(signed_record: SignedVoteRecord) -> bool {
        crate::signed_record::tests::prop_roundtrip(signed_record)
    }
}

// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

/// JSON based test in case we want to configure the Genesis by hand.
mod json {
    use fendermint_testing::golden_json;
    use fendermint_vm_genesis::Genesis;
    use quickcheck::Arbitrary;
    golden_json! { "genesis/json", genesis, Genesis::arbitrary }
}

/// CBOR based tests in case we have to grab Genesis from on-chain storage.
mod cbor {
    use fendermint_testing::golden_cbor;
    use fendermint_vm_genesis::Genesis;
    use quickcheck::Arbitrary;
    golden_cbor! { "genesis/cbor", genesis, Genesis::arbitrary }
}

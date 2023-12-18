// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: MIT

use ipc_sdk::subnet_id::SubnetID;
use serde_json::{json, Value};

/// A trait to convert a type to a JSON value.
pub(crate) trait ToJson {
    fn to_json(&self) -> Value;
}

/// Implement the `ToJson` trait for `SubnetID`.
impl ToJson for SubnetID {
    fn to_json(&self) -> Value {
        let children: Vec<String> = self
            .children_as_ref()
            .iter()
            .map(|addr| addr.to_string())
            .collect();
        json!({"Root": self.root_id(), "Children": children})
    }
}

#[cfg(test)]
#[test]
fn test_to_json() {
    use std::str::FromStr;
    let subnet_id = SubnetID::from_str("/r123/f0102").unwrap();
    let json = subnet_id.to_json();
    assert_eq!(
        json,
        json!({"Root": 123, "Children": vec!["f0102"]}),
        "subnet id to json failed"
    );
}

// Copyright 2022-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::error::ExitCode;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

impl From<anyhow::Error> for JsonRpcError {
    fn from(value: anyhow::Error) -> Self {
        Self {
            code: 0,
            message: format!("{:#}", value),
            data: None,
        }
    }
}

impl From<tendermint_rpc::Error> for JsonRpcError {
    fn from(value: tendermint_rpc::Error) -> Self {
        Self {
            code: 0,
            message: format!("Tendermint RPC error: {value}"),
            data: None,
        }
    }
}

impl From<JsonRpcError> for jsonrpc_v2::Error {
    fn from(value: JsonRpcError) -> Self {
        Self::Full {
            code: value.code,
            message: value.message,
            data: value.data.map(|d| {
                let d: Box<dyn erased_serde::Serialize + Send> = Box::new(d);
                d
            }),
        }
    }
}

pub fn error<T>(exit_code: ExitCode, msg: impl ToString) -> Result<T, JsonRpcError> {
    Err(JsonRpcError {
        code: exit_code.value().into(),
        message: msg.to_string(),
        data: None,
    })
}

pub fn error_with_data<T, E: Serialize>(
    exit_code: ExitCode,
    msg: impl ToString,
    data: E,
) -> Result<T, JsonRpcError> {
    let data = match serde_json::to_value(data) {
        Ok(v) => v,
        Err(e) => serde_json::Value::String(format!("failed to serialize error data: {e}")),
    };
    Err(JsonRpcError {
        code: exit_code.value().into(),
        message: msg.to_string(),
        data: Some(data),
    })
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code: {})", self.message, self.code)
    }
}

impl std::error::Error for JsonRpcError {}

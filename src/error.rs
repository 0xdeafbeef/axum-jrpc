use serde::Serialize;
use thiserror::Error;

/// Constants for [error object](https://www.jsonrpc.org/specification#error_object)
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;
pub const PARSE_ERROR: i32 = -32700;

#[derive(Debug)]
pub enum JsonRpcErrorReason {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    /// -32000 to -32099
    ServerError(i32),
}

impl std::fmt::Display for JsonRpcErrorReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonRpcErrorReason::ParseError => write!(f, "Parse error"),
            JsonRpcErrorReason::InvalidRequest => write!(f, "Invalid Request"),
            JsonRpcErrorReason::MethodNotFound => write!(f, "Method not found"),
            JsonRpcErrorReason::InvalidParams => write!(f, "Invalid params"),
            JsonRpcErrorReason::InternalError => write!(f, "Internal error"),
            JsonRpcErrorReason::ServerError(code) => write!(f, "Server error: {}", code),
        }
    }
}

impl From<JsonRpcErrorReason> for i32 {
    fn from(reason: JsonRpcErrorReason) -> i32 {
        match reason {
            JsonRpcErrorReason::ParseError => PARSE_ERROR,
            JsonRpcErrorReason::InvalidRequest => INVALID_REQUEST,
            JsonRpcErrorReason::MethodNotFound => METHOD_NOT_FOUND,
            JsonRpcErrorReason::InvalidParams => INVALID_PARAMS,
            JsonRpcErrorReason::InternalError => INTERNAL_ERROR,
            JsonRpcErrorReason::ServerError(code) => code,
        }
    }
}

impl JsonRpcErrorReason {
    fn new(code: i32) -> Self {
        match code {
            PARSE_ERROR => Self::ParseError,
            INVALID_REQUEST => Self::InvalidRequest,
            METHOD_NOT_FOUND => Self::MethodNotFound,
            INVALID_PARAMS => Self::InvalidParams,
            INTERNAL_ERROR => Self::InternalError,
            other => Self::ServerError(other),
        }
    }
}

#[derive(Debug, Error, Serialize)]
pub struct JsonRpcError {
    code: i32,
    message: String,
    data: serde_json::Value,
}

impl JsonRpcError {
    pub fn new(code: JsonRpcErrorReason, message: String, data: serde_json::Value) -> Self {
        Self {
            code: code.into(),
            message,
            data,
        }
    }
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}",
            JsonRpcErrorReason::new(self.code),
            self.message
        )
    }
}

#[cfg(feature = "anyhow_error")]
impl From<anyhow::Error> for JsonRpcError {
    fn from(error: anyhow::Error) -> Self {
        let message = error.to_string();
        let data = serde_json::Value::Null;
        Self {
            code: INTERNAL_ERROR,
            message,
            data,
        }
    }
}

impl JsonRpcError {
    pub fn error_reason(&self) -> JsonRpcErrorReason {
        JsonRpcErrorReason::new(self.code)
    }

    pub fn code(&self) -> i32 {
        self.code
    }
}

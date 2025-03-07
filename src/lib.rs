#![warn(
    clippy::all,
    clippy::dbg_macro,
    clippy::todo,
    clippy::empty_enum,
    clippy::enum_glob_use,
    clippy::mem_forget,
    clippy::unused_self,
    clippy::filter_map_next,
    clippy::needless_continue,
    clippy::needless_borrow,
    clippy::match_wildcard_for_single_variants,
    clippy::if_let_mutex,
    unexpected_cfgs,
    clippy::await_holding_lock,
    clippy::match_on_vec_items,
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    clippy::lossy_float_literal,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::fn_params_excessive_bools,
    clippy::exit,
    clippy::inefficient_to_string,
    clippy::linkedlist,
    clippy::macro_use_imports,
    clippy::option_option,
    clippy::verbose_file_reads,
    clippy::unnested_or_patterns,
    clippy::str_to_string,
    rust_2018_idioms,
    future_incompatible,
    nonstandard_style,
    missing_debug_implementations
)]
#![deny(unreachable_pub)]
#![allow(elided_lifetimes_in_paths, clippy::type_complexity)]

use std::borrow::Cow;

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use axum::Json;
use cfg_if::cfg_if;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

cfg_if! {
    if #[cfg(feature = "serde_json")] {
        pub use serde_json::Value;
        pub mod error;
        use crate::error::{JsonRpcError, JsonRpcErrorReason};
    }
    else if #[cfg(feature = "simd")] {
        pub use simd_json::OwnedValue as Value;
        pub mod error;
        use crate::error::{JsonRpcError, JsonRpcErrorReason};
    }
    else {
        compile_error!("features `serde_json` and `simd` are mutually exclusive");
    }
}

/// Hack until [try_trait_v2](https://github.com/rust-lang/rust/issues/84277) is not stabilized
pub type JrpcResult = Result<JsonRpcResponse, JsonRpcResponse>;

#[derive(Debug)]
pub struct JsonRpcRequest {
    pub id: Id,
    pub method: String,
    pub params: Value,
}

impl Serialize for JsonRpcRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Helper<'a> {
            jsonrpc: &'static str,
            id: Id,
            method: &'a str,
            params: &'a Value,
        }

        Helper {
            jsonrpc: JSONRPC,
            id: self.id.clone(),
            method: &self.method,
            params: &self.params,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsonRpcRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct Helper<'a> {
            #[serde(borrow)]
            jsonrpc: Cow<'a, str>,
            id: Id,
            method: String,
            params: Value,
        }

        let helper = Helper::deserialize(deserializer)?;
        if helper.jsonrpc == JSONRPC {
            Ok(Self {
                id: helper.id,
                method: helper.method,
                params: helper.params,
            })
        } else {
            Err(D::Error::custom("Unknown jsonrpc version"))
        }
    }
}

#[derive(Clone, Debug)]
/// Parses a JSON-RPC request, and returns the request ID, the method name, and the parameters.
/// If the request is invalid, returns an error.
/// ```rust
/// use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};
///
/// fn router(req: JsonRpcExtractor) -> JrpcResult {
///   let req_id = req.get_answer_id();
///   let method = req.method();
///   match method {
///     "add" => {
///        let params: [i32;2] = req.parse_params()?;
///        return Ok(JsonRpcResponse::success(req_id, params[0] + params[1]));
///     }
///     m =>  Ok(req.method_not_found(m))
///   }
/// }
/// ```
pub struct JsonRpcExtractor {
    pub parsed: Value,
    pub method: String,
    pub id: Id,
}

impl JsonRpcExtractor {
    pub fn get_answer_id(&self) -> Id {
        self.id.clone()
    }

    pub fn parse_params<T: DeserializeOwned>(self) -> Result<T, JsonRpcResponse> {
        cfg_if::cfg_if! {
           if #[cfg(feature = "simd")] {
                match simd_json::serde::from_owned_value(self.parsed){
                    Ok(v) => Ok(v),
                    Err(e) => {
                        let error = JsonRpcError::new(
                            JsonRpcErrorReason::InvalidParams,
                            e.to_string(),
                            Value::default(),
                        );
                        Err(JsonRpcResponse::error(self.id, error))
                    }

                }
            } else if #[cfg(feature = "serde_json")] {
                match serde_json::from_value(self.parsed){
                    Ok(v) => Ok(v),
                    Err(e) => {
                        let error = JsonRpcError::new(
                            JsonRpcErrorReason::InvalidParams,
                            e.to_string(),
                            Value::Null,
                        );
                        Err(JsonRpcResponse::error(self.id, error))
                    }
                }
            }
        }
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn method_not_found(&self, method: &str) -> JsonRpcResponse {
        let error = JsonRpcError::new(
            JsonRpcErrorReason::MethodNotFound,
            format!("Method `{}` not found", method),
            Value::default(),
        );

        JsonRpcResponse::error(self.id.clone(), error)
    }
}

impl<S> FromRequest<S> for JsonRpcExtractor
where
    Bytes: FromRequest<S>,
    S: Send + Sync,
{
    type Rejection = JsonRpcResponse;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !json_content_type(req.headers()) {
            return Err(JsonRpcResponse {
                id: Id::None(()),
                result: JsonRpcAnswer::Error(JsonRpcError::new(
                    JsonRpcErrorReason::InvalidRequest,
                    "Invalid content type".to_owned(),
                    Value::default(),
                )),
            });
        }

        #[allow(unused_mut)]
        let mut bytes = match Bytes::from_request(req, state).await {
            Ok(a) => a.to_vec(),
            Err(_) => {
                return Err(JsonRpcResponse {
                    id: Id::None(()),
                    result: JsonRpcAnswer::Error(JsonRpcError::new(
                        JsonRpcErrorReason::InvalidRequest,
                        "Invalid request".to_owned(),
                        Value::default(),
                    )),
                })
            }
        };

        cfg_if!(
            if #[cfg(feature = "simd")] {
               let parsed: JsonRpcRequest = match simd_json::from_slice(&mut bytes){
                    Ok(a) => a,
                    Err(e) => {
                        return Err(JsonRpcResponse {
                            id: Id::None(()),
                            result: JsonRpcAnswer::Error(JsonRpcError::new(
                                JsonRpcErrorReason::InvalidRequest,
                                e.to_string(),
                                Value::default(),
                            )),
                        })
                    }
                };
            } else if #[cfg(feature = "serde_json")] {
               let parsed: JsonRpcRequest = match serde_json::from_slice(&bytes){
                    Ok(a) => a,
                    Err(e) => {
                        return Err(JsonRpcResponse {
                            id: Id::None(()),
                            result: JsonRpcAnswer::Error(JsonRpcError::new(
                                JsonRpcErrorReason::InvalidRequest,
                                e.to_string(),
                                Value::default(),
                            )),
                        })
                    }
                };
            }
        );

        Ok(Self {
            parsed: parsed.params,
            method: parsed.method,
            id: parsed.id,
        })
    }
}

fn json_content_type(headers: &HeaderMap) -> bool {
    let content_type = if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        content_type
    } else {
        return false;
    };

    let content_type = if let Ok(content_type) = content_type.to_str() {
        content_type
    } else {
        return false;
    };

    let mime = if let Ok(mime) = content_type.parse::<mime::Mime>() {
        mime
    } else {
        return false;
    };

    let is_json_content_type = mime.type_() == "application"
        && (mime.subtype() == "json" || mime.suffix().map_or(false, |name| name == "json"));

    is_json_content_type
}

#[derive(Debug, Clone, PartialEq)]
/// A JSON-RPC response.
pub struct JsonRpcResponse {
    /// Request content.
    pub result: JsonRpcAnswer,
    /// The request ID.
    pub id: Id,
}

impl JsonRpcResponse {
    fn new<ID>(id: ID, result: JsonRpcAnswer) -> Self
    where
        Id: From<ID>,
    {
        Self {
            result,
            id: id.into(),
        }
    }

    /// Returns a response with the given result
    /// Returns JsonRpcError if the `result` is invalid input for [`serde_json::to_value`]
    pub fn success<T, ID>(id: ID, result: T) -> Self
    where
        T: Serialize,
        Id: From<ID>,
    {
        cfg_if::cfg_if! {
          if #[cfg(feature = "simd")] {
            match simd_json::serde::to_owned_value(result) {
                Ok(v) => JsonRpcResponse::new(id, JsonRpcAnswer::Result(v)),
                Err(e) => {
                    let err = JsonRpcError::new(
                        JsonRpcErrorReason::InternalError,
                        e.to_string(),
                        Value::default(),
                    );
                    JsonRpcResponse::error(id, err)
                }
            }
          } else if #[cfg(feature = "serde_json")] {
            match serde_json::to_value(result) {
                Ok(v) => JsonRpcResponse::new(id, JsonRpcAnswer::Result(v)),
                Err(e) => {
                    let err = JsonRpcError::new(
                        JsonRpcErrorReason::InternalError,
                        e.to_string(),
                        Value::Null,
                    );
                    JsonRpcResponse::error(id, err)
                }
            }
          }
        }
    }

    pub fn error<ID>(id: ID, error: JsonRpcError) -> Self
    where
        Id: From<ID>,
    {
        let id = id.into();
        JsonRpcResponse {
            result: JsonRpcAnswer::Error(error),
            id,
        }
    }
}

impl Serialize for JsonRpcResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Helper<'a> {
            jsonrpc: &'static str,
            #[serde(flatten)]
            result: &'a JsonRpcAnswer,
            id: Id,
        }

        Helper {
            jsonrpc: JSONRPC,
            result: &self.result,
            id: self.id.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsonRpcResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct Helper<'a> {
            #[serde(borrow)]
            jsonrpc: Cow<'a, str>,
            #[serde(flatten)]
            result: JsonRpcAnswer,
            id: Id,
        }

        let helper = Helper::deserialize(deserializer)?;
        if helper.jsonrpc == JSONRPC {
            Ok(Self {
                result: helper.result,
                id: helper.id,
            })
        } else {
            Err(D::Error::custom("Unknown jsonrpc version"))
        }
    }
}

impl IntoResponse for JsonRpcResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

#[derive(Serialize, Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
/// JsonRpc [response object](https://www.jsonrpc.org/specification#response_object)
pub enum JsonRpcAnswer {
    Result(Value),
    Error(JsonRpcError),
}

const JSONRPC: &str = "2.0";

/// An identifier established by the Client that MUST contain a String, Number,
/// or NULL value if included. If it is not included it is assumed to be a notification.
/// The value SHOULD normally not be Null and Numbers SHOULD NOT contain fractional parts
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize, Hash)]
#[serde(untagged)]
pub enum Id {
    Num(i64),
    Str(String),
    None(()),
}

impl From<()> for Id {
    fn from(val: ()) -> Self {
        Id::None(val)
    }
}

impl From<i64> for Id {
    fn from(val: i64) -> Self {
        Id::Num(val)
    }
}

impl From<String> for Id {
    fn from(val: String) -> Self {
        Id::Str(val)
    }
}

#[cfg(test)]
#[cfg(all(feature = "anyhow_error", feature = "serde_json"))]
mod test {
    use crate::{
        Deserialize, JrpcResult, JsonRpcAnswer, JsonRpcError, JsonRpcErrorReason, JsonRpcExtractor,
        JsonRpcRequest, JsonRpcResponse,
    };
    use axum::routing::post;
    use serde::Serialize;
    use serde_json::Value;

    #[tokio::test]
    async fn test() {
        use axum::http::StatusCode;
        use axum::Router;
        use axum_test::TestServer;

        // you can replace this Router with your own app
        let app = Router::new().route("/", post(handler));

        // initiate the TestClient with the previous declared Router
        let client = TestServer::new(app).unwrap();

        let res = client
            .post("/")
            .json(&JsonRpcRequest {
                id: 0.into(),
                method: "add".to_owned(),
                params: serde_json::to_value(Test { a: 0, b: 111 }).unwrap(),
            })
            .await;
        assert_eq!(res.status_code(), StatusCode::OK);
        let response = res.json::<JsonRpcResponse>();
        assert_eq!(response.result, JsonRpcAnswer::Result(111.into()));

        let res = client
            .post("/")
            .json(&JsonRpcRequest {
                id: 0.into(),
                method: "lol".to_owned(),
                params: serde_json::to_value(()).unwrap(),
            })
            .await;

        assert_eq!(res.status_code(), StatusCode::OK);

        let response = res.json::<JsonRpcResponse>();

        let error = JsonRpcError::new(
            JsonRpcErrorReason::MethodNotFound,
            format!("Method `{}` not found", "lol"),
            Value::Null,
        );

        let error = JsonRpcResponse::error(0, error);

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::to_value(response).unwrap()
        );
    }

    async fn handler(value: JsonRpcExtractor) -> JrpcResult {
        let answer_id = value.get_answer_id();
        println!("{:?}", value);
        match value.method.as_str() {
            "add" => {
                let request: Test = value.parse_params()?;
                let result = request.a + request.b;
                Ok(JsonRpcResponse::success(answer_id, result))
            }
            "sub" => {
                let result: [i32; 2] = value.parse_params()?;
                let result = match failing_sub(result[0], result[1]).await {
                    Ok(result) => result,
                    Err(e) => return Err(JsonRpcResponse::error(answer_id, e.into())),
                };
                Ok(JsonRpcResponse::success(answer_id, result))
            }
            "div" => {
                let result: [i32; 2] = value.parse_params()?;
                let result = match failing_div(result[0], result[1]).await {
                    Ok(result) => result,
                    Err(e) => return Err(JsonRpcResponse::error(answer_id, e.into())),
                };

                Ok(JsonRpcResponse::success(answer_id, result))
            }
            method => Ok(value.method_not_found(method)),
        }
    }

    async fn failing_sub(a: i32, b: i32) -> anyhow::Result<i32> {
        anyhow::ensure!(a > b, "a must be greater than b");
        Ok(a - b)
    }

    async fn failing_div(a: i32, b: i32) -> Result<i32, CustomError> {
        if b == 0 {
            Err(CustomError::DivideByZero)
        } else {
            Ok(a / b)
        }
    }

    #[derive(Deserialize, Serialize, Debug)]
    struct Test {
        a: i32,
        b: i32,
    }

    #[derive(Debug, thiserror::Error)]
    enum CustomError {
        #[error("Divisor must not be equal to 0")]
        DivideByZero,
    }

    impl From<CustomError> for JsonRpcError {
        fn from(error: CustomError) -> Self {
            JsonRpcError::new(
                JsonRpcErrorReason::ServerError(-32099),
                error.to_string(),
                serde_json::Value::Null,
            )
        }
    }
}

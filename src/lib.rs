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
    clippy::mismatched_target_os,
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
#![deny(unreachable_pub, private_in_public)]
#![allow(elided_lifetimes_in_paths, clippy::type_complexity)]

use std::borrow::Cow;

use axum::body::HttpBody;
use axum::extract::FromRequest;
use axum::http::Request;
use axum::response::{IntoResponse, Response};
use axum::{BoxError, Json};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{JsonRpcError, JsonRpcErrorReason};

pub mod error;

/// Hack until [try_trait_v2](https://github.com/rust-lang/rust/issues/84277) is not stabilized
pub type JrpcResult = Result<JsonRpcResponse, JsonRpcResponse>;

#[derive(Debug)]
pub struct JsonRpcRequest {
    pub id: i64,
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
            id: i64,
            method: &'a str,
            params: &'a Value,
        }

        Helper {
            jsonrpc: JSONRPC,
            id: self.id,
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
            id: i64,
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

#[derive(Debug)]
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
    pub id: i64,
}

impl JsonRpcExtractor {
    pub fn get_answer_id(&self) -> i64 {
        self.id
    }

    pub fn parse_params<T: DeserializeOwned>(self) -> Result<T, JsonRpcResponse> {
        let value = serde_json::from_value(self.parsed);
        match value {
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

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn method_not_found(&self, method: &str) -> JsonRpcResponse {
        let error = JsonRpcError::new(
            JsonRpcErrorReason::MethodNotFound,
            format!("Method `{}` not found", method),
            Value::Null,
        );

        JsonRpcResponse::error(self.id, error)
    }
}

#[async_trait::async_trait]
impl<S, B> FromRequest<S, B> for JsonRpcExtractor
where
    B: HttpBody + Send + 'static,
    B::Data: Send,
    B::Error: Into<BoxError>,
    S: Send + Sync,
{
    type Rejection = JsonRpcResponse;

    async fn from_request(req: Request<B>, state: &S) -> Result<Self, Self::Rejection> {
        let json = Json::from_request(req, state).await;
        let parsed: JsonRpcRequest = match json {
            Ok(a) => a.0,
            Err(e) => {
                return Err(JsonRpcResponse {
                    id: 0,
                    result: JsonRpcAnswer::Error(JsonRpcError::new(
                        JsonRpcErrorReason::InvalidRequest,
                        e.to_string(),
                        Value::Null,
                    )),
                })
            }
        };

        Ok(Self {
            parsed: parsed.params,
            method: parsed.method,
            id: parsed.id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A JSON-RPC response.
pub struct JsonRpcResponse {
    /// Request content.
    pub result: JsonRpcAnswer,
    /// The request ID.
    pub id: i64,
}

impl JsonRpcResponse {
    fn new(id: i64, result: JsonRpcAnswer) -> Self {
        Self { result, id }
    }

    /// Returns a response with the given result
    /// Returns JsonRpcError if the `result` is invalid input for [`serde_json::to_value`]
    pub fn success<T: Serialize>(id: i64, result: T) -> Self {
        let result = match serde_json::to_value(result) {
            Ok(v) => v,
            Err(e) => {
                let err = JsonRpcError::new(
                    JsonRpcErrorReason::InternalError,
                    e.to_string(),
                    Value::Null,
                );
                return JsonRpcResponse::error(id, err);
            }
        };

        JsonRpcResponse::new(id, JsonRpcAnswer::Result(result))
    }

    pub fn error(id: i64, error: JsonRpcError) -> Self {
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
            id: i64,
        }

        Helper {
            jsonrpc: JSONRPC,
            result: &self.result,
            id: self.id,
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
            id: i64,
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

#[derive(Serialize, Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
/// JsonRpc [response object](https://www.jsonrpc.org/specification#response_object)
pub enum JsonRpcAnswer {
    Result(Value),
    Error(JsonRpcError),
}

const JSONRPC: &str = "2.0";

#[cfg(test)]
#[cfg(feature = "anyhow_error")]
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
        use axum_test_helper::TestClient;

        // you can replace this Router with your own app
        let app = Router::new().route("/", post(handler));

        // initiate the TestClient with the previous declared Router
        let client = TestClient::new(app);

        let res = client
            .post("/")
            .json(&JsonRpcRequest {
                id: 0,
                method: "add".to_owned(),
                params: serde_json::to_value(Test { a: 0, b: 111 }).unwrap(),
            })
            .send()
            .await;
        assert_eq!(res.status(), StatusCode::OK);
        let response = res.json::<JsonRpcResponse>().await;
        assert_eq!(response.result, JsonRpcAnswer::Result(111.into()));

        let res = client
            .post("/")
            .json(&JsonRpcRequest {
                id: 0,
                method: "lol".to_owned(),
                params: serde_json::to_value(()).unwrap(),
            })
            .send()
            .await;

        assert_eq!(res.status(), StatusCode::OK);

        let response = res.json::<JsonRpcResponse>().await;

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

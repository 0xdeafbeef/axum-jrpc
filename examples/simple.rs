use axum::routing::post;
use axum::Router;
use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};

use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use axum_jrpc::Value;
use serde::Deserialize;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let router = Router::new().route("/", post(handler));

    tracing::debug!("listening on 127.0.0.1:8080");
    axum::Server::bind(&"127.0.0.1:8080".parse().unwrap())
        .serve(router.into_make_service())
        .await
        .unwrap();
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

#[derive(Deserialize, Debug)]
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
            Value::default(),
        )
    }
}

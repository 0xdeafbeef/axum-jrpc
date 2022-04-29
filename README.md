# Json RPC extractor for axum


`JsonRpcExtractor` parses JSON-RPC requests and validates it's correctness.

```rust
use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcRepsonse};

fn router(req: JsonRpcExtractor) -> JrpcResult {
    let req_id = req.get_answer_id()?;
    let method = req.method();
    let response = 
    match method {
        "add" => {
            let params: [i32; 2] = req.parse_params()?;
            JsonRpcRepsonse::success(req_id, params[0] + params[1]);
        }
        m => req.method_not_found(m)
    };
    
    Ok(response)
}
```

![docs.rs](https://img.shields.io/docsrs/axum-jrpc?style=plastic)
![Crates.io](https://img.shields.io/crates/l/axum-jrpc)
# Tubetti: small http tubes

No-fuss, low configuration webservers on demand 

## Features:
- Convenience-focused development experience
- Axum-based, can optionally be constructed from a [Router](https://docs.rs/axum/latest/axum/struct.Router.html)
- Supports ranged requests using [axum-range](https://github.com/haileys/axum-range)
- Supports custom headers using [HeaderMap](https://docs.rs/http/1.2.0/http/header/struct.HeaderMap.html)
- Supports artifically slow response times with configurable delay

## Example Usage
```rust 
    use tubetti::tube;

    let body = std::sync::Arc::new("potatoes".as_bytes());
    let tb = tube!(body).await.unwrap();
    let client = reqwest::Client::new();
    let response = client.get(tb.url()).send().await.unwrap();
    assert_eq!(response.headers().get("accept-ranges").unwrap(), "bytes");
    assert_eq!(response.headers().get("content-length").unwrap(), "8");
    assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
    tb.shutdown().await.unwrap();
```

Please take a look at the examples for more detailed usage information.

## Caveats:
- Under active development, expect breaking changes per-release until stable

## License

MIT OR Apache-2.0
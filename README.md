# Tubetti: small http tubes

No-fuss, low configuration webservers on demand 

## Features:
- Convenience-focused development experience
- Axum-based, can optionally be constructed from a [Router](https://docs.rs/axum/latest/axum/struct.Router.html)
- Supports ranged requests using [axum-range](https://github.com/haileys/axum-range)
- Supports custom headers using [HeaderMap](https://docs.rs/http/1.2.0/http/header/struct.HeaderMap.html)

### Example Usage

```rust
        // most convenient, just give it some bytes and they're served on a random port
        let tb = tube!("potatoes".as_bytes()).await.unwrap();
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
        tb.shutdown().await.unwrap();

        // with port
        let tb = tube!("potatoes".as_bytes(), 6301).await.unwrap();
        assert_eq!(tb.url(), "http://0.0.0.0:6301".to_string());
        tb.shutdown().await.unwrap();

        // with port and status
        let tb = tube!("potatoes".as_bytes(), 6301, StatusCode::BAD_GATEWAY)
            .await
            .unwrap();
        assert_eq!(tb.url(), "http://0.0.0.0:6301".to_string());
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.status(), 502);
        assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
        tb.shutdown().await.unwrap();

        // with port, status and headers
        let mut headers = crate::axum::http::HeaderMap::new();
        headers.append("pasta", crate::axum::http::HeaderValue::from_static("yum"));
        let tb = tube!(
            "potatoes".as_bytes(),
            6301,
            StatusCode::BAD_GATEWAY,
            headers
        )
        .await
        .unwrap();
        assert_eq!(tb.url(), "http://0.0.0.0:6301".to_string());
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.status(), 502);
        assert_eq!(response.headers().get("pasta").unwrap(), "yum");
        assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
        tb.shutdown().await.unwrap();
```

## Caveats:
- Under active development, expect breaking changes per-release until stable

## License

MIT OR Apache-2.0
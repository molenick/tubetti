# Tubetti: small http tubes

Serve &[u8] data at a localhost url with minimal configuration.

## Features:
- Convenience-focused development experience
- Axum-based, can optionally be constructed from a [Router](https://docs.rs/axum/latest/axum/struct.Router.html)

## Caveats:
- Under active development, expect breaking changes per-release until stable

## Example usage:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;

    #[tokio::test]
    /// Proves ranged response behavior
    async fn test_range_request() {
        let tb = tube!("happy valentine's day".as_bytes()).await;

        // a request without range metadata specified
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.bytes().await.unwrap(),
            "happy valentine's day".as_bytes()
        );

        // a request with valid range metadata specified
        let response = client
            .get(tb.url())
            .header("Range", "bytes=0-4")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(response.bytes().await.unwrap(), "happy".as_bytes());

        // a request with invalid range metadata specified
        let response = client
            .get(tb.url())
            .header("Range", "bytes=15-22")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 416);
        assert_eq!(response.bytes().await.unwrap(), "".as_bytes());
    }

    #[tokio::test]
    /// Proves header injection
    async fn test_header_injection() {
        let mut headers = crate::axum::http::HeaderMap::new();
        headers.append("pasta", crate::axum::http::HeaderValue::from_static("yum"));
        let body = "happy valentine's day".as_bytes();
        let app = crate::axum::Router::new()
            .route("/", crate::axum::routing::get(Tube::serve_ranged_request))
            .with_state((Body::new(body), headers));
        let tb = Tube::new(app, None).await;

        let client = Client::new();
        let response = client
            .get(tb.url())
            .header("Range", "bytes=0-0")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
        assert_eq!(response.headers().get("pasta").unwrap(), "yum");
        assert_eq!(response.headers().get("content-length").unwrap(), "1");
        assert!(response.headers().get("date").is_some());
        assert_eq!(response.bytes().await.unwrap(), "h".as_bytes());
    }

    #[tokio::test]
    /// Proves custom port binding
    async fn test_port_binding() {
        let headers = crate::axum::http::HeaderMap::new();
        let body = "happy valentine's day".as_bytes();
        let app = crate::axum::Router::new()
            .route("/", crate::axum::routing::get(Tube::serve_ranged_request))
            .with_state((Body::new(body), headers));
        let _tb = Tube::new(app, Some(8899)).await;

        let client = Client::new();
        let response = client
            .get("http://127.0.0.1:8899")
            .header("Range", "bytes=0-0")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206);
    }
}

```

## License

MIT OR Apache-2.0
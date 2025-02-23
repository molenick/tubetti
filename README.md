# Tubetti: small http tubes

No-fuss, low configuration webservers on demand 

## Features:
- Convenience-focused development experience
- Axum-based, can optionally be constructed from a [Router](https://docs.rs/axum/latest/axum/struct.Router.html)
- Supports ranged requests using [axum-range](https://github.com/haileys/axum-range)
- Supports custom headers using [HeaderMap](https://docs.rs/http/1.2.0/http/header/struct.HeaderMap.html)

### Example Usage

```rust
        // most convenient, configure a tube with just bytes on a randomly
        // selected unused port:
        let _tb = tube!("potatoes".as_bytes()).await.unwrap();

        // configure a tube with bytes, custom headers and port:
        let _tb_opt = tube!("tomatoes".as_bytes(), HeaderMap::new(), 2323).await.unwrap();

        // "advanced configuration": use the construct w/ a hand-rolled
        // axum app:
        let mut headers = crate::axum::http::HeaderMap::new();
        headers.append("pasta", crate::axum::http::HeaderValue::from_static("yum"));
        let body = "happy valentine's day".as_bytes();
        let app = crate::axum::Router::new()
            .route("/", crate::axum::routing::get(Tube::serve_ranged_request))
            .with_state((Body::new(body), headers));
        let majestic_tubes = Tube::new(app, Some(2323)).await.unwrap();

`

## Caveats:
- Under active development, expect breaking changes per-release until stable

## Example usage:


## License

MIT OR Apache-2.0
# Tubetti: small http tubes

Serve &[u8] data at a localhost url with minimal configuration.

## Features:
- Convenience-focused development experience
- Axum-based, can optionally be constructed from a [Router](https://docs.rs/axum/latest/axum/struct.Router.html)

## Example usage:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_range_request() {
        let tube = tube!("happy valentine's day".as_bytes()).await;

        let client = reqwest::Client::new();
        let response = client.get(tube.url()).send().await.unwrap();

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.bytes().await.unwrap(),
            "happy valentine's day".as_bytes()
        );

        let response = client
            .get(tube.url())
            .header("Range", "bytes=500-600")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 416);
        assert_eq!(response.bytes().await.unwrap(), "".as_bytes());
    }
}
```

## License

MIT OR Apache-2.0
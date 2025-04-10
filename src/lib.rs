use std::task::{Context, Poll};

use axum::http::HeaderMap;
use axum_range::AsyncSeekStart;
use bytes::BufMut;

use error::Error;

use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::oneshot,
};

pub mod error {
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("shutdown failed")]
        ShutdownFailed,
        #[error(transparent)]
        Io(#[from] std::io::Error),
    }
}

// Re-export axum for optional use/convenience
pub use axum;

/// A `Vec<u8>` wrapper that implements pre-conditions for `axum_range::KnownSize`
#[derive(Clone)]
pub struct Body {
    data: Vec<u8>,
    seek_position: u64,
}
impl Body {
    pub fn new(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            seek_position: 0,
        }
    }
}

/// Precondition for impl axum_range::KnownSize
impl AsyncRead for Body {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let data = &self.data;
        let seek_position = self.seek_position as usize;

        let remaining = data.len() - seek_position;

        if remaining > 0 {
            let n = std::cmp::min(remaining, buf.remaining_mut());
            buf.put_slice(&data[seek_position..seek_position + n]);
            Poll::Ready(Ok(()))
        } else {
            Poll::Ready(Ok(()))
        }
    }
}

/// Precondition for impl axum_range::KnownSize
impl AsyncSeekStart for Body {
    fn start_seek(self: std::pin::Pin<&mut Self>, position: u64) -> std::io::Result<()> {
        let s = self.get_mut();
        s.seek_position = position;
        Ok(())
    }

    fn poll_complete(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Constructing a Tube spins up an axum webserver. The resulting
/// object contains server metadata and control.
pub struct Tube {
    shutdown_tx: oneshot::Sender<()>,
    url: String,
}

impl Tube {
    /// Endpoint convenience constructor pre-configured to serve ranged requests
    pub async fn new_range_response_server(body: &[u8]) -> Result<Self, Error> {
        let app = crate::axum::Router::new()
            .route(
                "/",
                crate::axum::routing::get(Self::serve_ranged_status_response),
            )
            .with_state((Body::new(body), None, None));
        Self::new(app, None).await
    }

    pub async fn new_range_response_server_opt(
        body: &[u8],
        headers: HeaderMap,
        port: u16,
    ) -> Result<Self, Error> {
        let app = crate::axum::Router::new()
            .route(
                "/",
                crate::axum::routing::get(Self::serve_ranged_status_response),
            )
            .with_state((Body::new(body), None, Some(headers)));

        Self::new(app, Some(port)).await
    }

    pub async fn status_response_server(status: axum::http::StatusCode) -> Result<Self, Error> {
        let app = crate::axum::Router::new().route(
            "/",
            crate::axum::routing::get(Self::serve_ranged_status_response).with_state((
                Body::new(&[]),
                Some(status),
                Some(HeaderMap::new()),
            )),
        );

        Self::new(app, None).await
    }

    /// The "advanced" endpoint constructor uses an Axum Router for its configuration
    pub async fn new(app: crate::axum::Router, port: Option<u16>) -> Result<Self, Error> {
        let port = port.unwrap_or(0);
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let addr = format!("http://{}", listener.local_addr()?);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            crate::axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .expect("Failed to serve tubetti");
        });

        Ok(Self {
            shutdown_tx,
            url: addr,
        })
    }

    /// Shutdown the endpoint
    pub fn shutdown(self) -> Result<(), Error> {
        self.shutdown_tx.send(()).map_err(|_| Error::ShutdownFailed)
    }

    /// Returns the url of the endpoint
    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub async fn serve_ranged_status_response(
        crate::axum::extract::State((body, status, headers)): crate::axum::extract::State<(
            Body,
            Option<axum::http::StatusCode>,
            Option<crate::axum::http::HeaderMap>,
        )>,
        range: Option<axum_extra::TypedHeader<axum_extra::headers::Range>>,
    ) -> impl crate::axum::response::IntoResponse {
        let len = body.data.len() as u64;
        let body = axum_range::KnownSize::sized(body, len);

        let range = range.map(|axum_extra::TypedHeader(range)| range);
        let mut response = crate::axum::response::IntoResponse::into_response(
            axum_range::Ranged::new(range, body),
        );

        match (status, headers) {
            (None, None) => response,
            (None, Some(mut headers)) => {
                std::mem::swap(&mut headers, response.headers_mut());

                let _ = headers
                    .drain()
                    .map(|h| response.headers_mut().append(h.0.unwrap(), h.1));
                response
            }
            (Some(status), None) => {
                *response.status_mut() = status;
                response
            }
            (Some(status), Some(mut headers)) => {
                *response.status_mut() = status;

                std::mem::swap(&mut headers, response.headers_mut());

                let _ = headers
                    .drain()
                    .map(|h| response.headers_mut().append(h.0.unwrap(), h.1));
                response
            }
        }
    }
}

/// Convenience macro for getting a ranged request Tube
#[macro_export]
macro_rules! tube {
    ($body:expr) => {
        Tube::new_range_response_server($body)
    };
    ($body:expr, $headers:expr, $port:expr) => {
        Tube::new_range_response_server_opt($body, $headers, $port)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::{Client, StatusCode};

    #[tokio::test]
    /// Proves ranged response behavior
    async fn test_range_request() {
        let tb = tube!("happy valentine's day".as_bytes()).await.unwrap();

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
            .route(
                "/",
                crate::axum::routing::get(Tube::serve_ranged_status_response),
            )
            .with_state((Body::new(body), None, Some(headers)));
        let tb = Tube::new(app, None).await.unwrap();

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
            .route(
                "/",
                crate::axum::routing::get(Tube::serve_ranged_status_response),
            )
            .with_state((Body::new(body), None, Some(headers)));
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

    #[tokio::test]
    /// Prove macros work
    async fn test_tube_macros() {
        // most convenient, just give it some bytes and they're served on a random port
        let _tb = tube!("potatoes".as_bytes()).await.unwrap();
        // most powerful, give it some bytes, some headers and a port
        let _tb_opt = tube!("tomatoes".as_bytes(), HeaderMap::new(), 2323)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_status_responses() {
        let tb = Tube::status_response_server(StatusCode::FAILED_DEPENDENCY)
            .await
            .unwrap();

        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.status(), 424);
        assert_eq!(response.content_length(), Some(0));
        assert!(response.headers().get("date").is_some());
        assert_eq!(response.bytes().await.unwrap(), vec![]);
    }
}

use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use axum::http::StatusCode;
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
        ShutdownInitFailed,
        #[error("shutdown confirmation failed: {0}")]
        ShutdownConfirmFailed(#[from] tokio::sync::oneshot::error::RecvError),
        #[error(transparent)]
        Io(#[from] std::io::Error),
    }
}

// Re-export axum for optional use/convenience
pub use axum;

/// A `Vec<u8>` wrapper that implements pre-conditions for `axum_range::KnownSize`
#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct TubeConfig {
    body: Body,
    /// Note: if you want to support real ranged requests, leave this
    /// as None. This is intended for non-success code simualation,
    /// overriding with success codes is unsupported for now.
    status: Option<StatusCode>,
    /// Note: allows addition of new headers but does not allow mutation
    /// of existing headers set by axum. Things like ranged request headers
    /// etc.
    headers: Option<HeaderMap>,
    delay: Duration,
}

/// Constructing a Tube spins up an axum webserver. The resulting
/// object contains server metadata and control.
pub struct Tube {
    shutdown_init_tx: oneshot::Sender<()>,
    shutdown_confirm_rx: oneshot::Receiver<()>,
    url: String,
}

impl Tube {
    /// The "convenience" endpoint constructor. Provide body and optional port, status, headers
    pub async fn new_ranged_status_response_server(
        body: &[u8],
        port: Option<u16>,
        status: Option<StatusCode>,
        headers: Option<HeaderMap>,
        delay: Option<Duration>,
    ) -> Result<Self, Error> {
        let config = TubeConfig {
            body: Body::new(body),
            status,
            headers,
            delay: delay.unwrap_or_default(),
        };
        let app = crate::axum::Router::new()
            .route(
                "/",
                crate::axum::routing::get(Self::serve_ranged_status_response),
            )
            .with_state(config);

        Self::new(app, port).await
    }

    /// The "advanced" endpoint constructor uses an Axum Router for its configuration
    pub async fn new(app: crate::axum::Router, port: Option<u16>) -> Result<Self, Error> {
        let port = port.unwrap_or(0);
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let addr = format!("http://{}", listener.local_addr()?);

        let (shutdown_init_tx, shutdown_init_rx) = oneshot::channel::<()>();
        let (shutdown_confirm_tx, shutdown_confirm_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            crate::axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_init_rx.await.expect("dirty shutdown");
                    shutdown_confirm_tx
                        .send(())
                        .expect("failed to confirm shutdown");
                })
                .await
                .expect("Failed to serve tubetti");
        });

        Ok(Self {
            shutdown_init_tx,
            shutdown_confirm_rx,
            url: addr,
        })
    }

    /// Shutdown the endpoint
    pub async fn shutdown(self) -> Result<(), Error> {
        self.shutdown_init_tx
            .send(())
            .map_err(|_| Error::ShutdownInitFailed)?;

        self.shutdown_confirm_rx.await?;

        Ok(())
    }

    /// Returns the url of the endpoint
    pub fn url(&self) -> String {
        self.url.clone()
    }

    pub async fn serve_ranged_status_response(
        crate::axum::extract::State(config): crate::axum::extract::State<TubeConfig>,
        range: Option<axum_extra::TypedHeader<axum_extra::headers::Range>>,
    ) -> impl crate::axum::response::IntoResponse {
        // track when the request began so that we can adjust delay in
        // an attempt to provide as precise of a request duration as
        // possible to the request value.
        let init_at = Instant::now();

        let len = config.body.data.len() as u64;
        let body = axum_range::KnownSize::sized(config.body, len);

        let range = range.map(|axum_extra::TypedHeader(range)| range);
        let mut response = crate::axum::response::IntoResponse::into_response(
            axum_range::Ranged::new(range, body),
        );

        let response = match (config.status, config.headers) {
            (None, None) => response,
            (None, Some(headers)) => {
                for header in headers {
                    response.headers_mut().append(header.0.unwrap(), header.1);
                }

                response
            }
            (Some(status), None) => {
                *response.status_mut() = status;
                response
            }
            (Some(status), Some(headers)) => {
                *response.status_mut() = status;

                for header in headers {
                    response.headers_mut().append(header.0.unwrap(), header.1);
                }

                response
            }
        };

        let elapsed = init_at.elapsed();
        let remaining_delay = config.delay.saturating_sub(elapsed);

        tokio::time::sleep(remaining_delay).await;

        response
    }
}

/// Convenience macro for getting a ranged request Tube
#[macro_export]
macro_rules! tube {
    ($body:expr) => {
        Tube::new_ranged_status_response_server($body, None, None, None, None)
    };
    ($body:expr, $port:expr) => {
        Tube::new_ranged_status_response_server($body, $port, None, None, None)
    };
    ($body:expr, $port:expr, $status:expr) => {
        Tube::new_ranged_status_response_server($body, $port, $status, None, None)
    };
    ($body:expr, $port:expr, $status:expr, $headers:expr) => {
        Tube::new_ranged_status_response_server($body, $port, $status, $headers, None)
    };
    ($body:expr, $port:expr, $status:expr, $headers:expr, $delay:expr) => {
        Tube::new_ranged_status_response_server($body, $port, $status, $headers, $delay)
    };
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use reqwest::{Client, StatusCode};

    #[tokio::test]
    /// Proves ranged response behavior
    async fn test_range_request() {
        let tb = tube!(
            "happy valentine's day".as_bytes(),
            None,
            None,
            Some(HeaderMap::new())
        )
        .await
        .unwrap();

        // a request without range metadata specified
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.status(), 200);

        assert_eq!(response.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(response.headers().get("content-length").unwrap(), "21");
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
    /// Prove macros work
    async fn test_tube_macros() {
        // most convenient, just give it some bytes and they're served on a random port
        let tb = tube!("potatoes".as_bytes()).await.unwrap();
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(response.headers().get("content-length").unwrap(), "8");
        assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
        tb.shutdown().await.unwrap();

        // with port
        let tb = tube!("potatoes".as_bytes(), Some(6301)).await.unwrap();
        assert_eq!(tb.url(), "http://0.0.0.0:6301".to_string());
        tb.shutdown().await.unwrap();

        // with port and status
        let tb = tube!(
            "potatoes".as_bytes(),
            Some(6301),
            Some(StatusCode::BAD_GATEWAY)
        )
        .await
        .unwrap();
        assert_eq!(tb.url(), "http://0.0.0.0:6301".to_string());
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(response.headers().get("content-length").unwrap(), "8");
        assert_eq!(response.status(), 502);
        assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
        tb.shutdown().await.unwrap();

        // with port, status and headers
        let mut headers = crate::axum::http::HeaderMap::new();
        headers.append("pasta", crate::axum::http::HeaderValue::from_static("yum"));
        let tb = tube!(
            "potatoes".as_bytes(),
            Some(6301),
            Some(StatusCode::BAD_GATEWAY),
            Some(headers)
        )
        .await
        .unwrap();
        assert_eq!(tb.url(), "http://0.0.0.0:6301".to_string());
        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();
        assert_eq!(response.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(response.headers().get("content-length").unwrap(), "8");
        assert_eq!(response.status(), 502);
        assert_eq!(response.headers().get("pasta").unwrap(), "yum");
        assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
        tb.shutdown().await.unwrap();

        // with port, status, headers, and delay
        let mut headers = crate::axum::http::HeaderMap::new();
        headers.append("pasta", crate::axum::http::HeaderValue::from_static("yum"));
        let delay = std::time::Duration::from_millis(200);
        let tb = tube!(
            "potatoes".as_bytes(),
            Some(6901),
            Some(StatusCode::BAD_GATEWAY),
            Some(headers),
            Some(delay)
        )
        .await
        .unwrap();
        assert_eq!(tb.url(), "http://0.0.0.0:6901".to_string());
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let start = Instant::now();
        let response = client.get(tb.url()).send().await.unwrap();
        assert!(Instant::now() - start >= delay);
        assert_eq!(response.headers().get("accept-ranges").unwrap(), "bytes");
        assert_eq!(response.headers().get("content-length").unwrap(), "8");
        assert_eq!(response.status(), 502);
        assert_eq!(response.headers().get("pasta").unwrap(), "yum");
        assert_eq!(response.bytes().await.unwrap(), "potatoes".as_bytes());
        tb.shutdown().await.unwrap();
    }
}

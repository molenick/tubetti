use std::task::{Context, Poll};

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
    ) -> Result<Self, Error> {
        let app = crate::axum::Router::new()
            .route(
                "/",
                crate::axum::routing::get(Self::serve_ranged_status_response),
            )
            .with_state((Body::new(body), status, headers));

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
        Tube::new_ranged_status_response_server($body, None, None, None)
    };
    ($body:expr, $port:expr) => {
        Tube::new_ranged_status_response_server($body, Some($port), None, None)
    };
    ($body:expr, $port:expr, $status:expr) => {
        Tube::new_ranged_status_response_server($body, Some($port), Some($status), None)
    };
    ($body:expr, $port:expr, $status:expr, $headers:expr) => {
        Tube::new_ranged_status_response_server($body, Some($port), Some($status), Some($headers))
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
    /// Prove macros work
    async fn test_tube_macros() {
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
    }
}

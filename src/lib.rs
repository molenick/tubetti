use std::task::{Context, Poll};

use axum::Router;
use axum_range::AsyncSeekStart;
use bytes::BufMut;

use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::oneshot,
};

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
    pub async fn new_range_request_server(body: &[u8]) -> Self {
        let app = Router::new()
            .route("/", axum::routing::get(Self::serve_ranged_request))
            .with_state(Body::new(body));
        Self::new(app).await
    }

    /// The "advanced" endpoint constructor uses an Axum Router for its configuration
    pub async fn new(app: Router) -> Self {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("Failed to bind to address {addr}");
        let addr = format!(
            "http://{}",
            listener.local_addr().expect("Failed to get local address")
        );

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .expect("Failed to serve tubetti");
        });

        Self {
            shutdown_tx,
            url: addr,
        }
    }

    /// Shutdown the endpoint
    pub fn shutdown(self) {
        self.shutdown_tx.send(()).expect("shutdown failed");
    }

    /// Returns the url of the endpoint
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Convenience configuration for an axum router that serves ranged
    /// requests
    async fn serve_ranged_request(
        axum::extract::State(state): axum::extract::State<Body>,
        range: Option<axum_extra::TypedHeader<axum_extra::headers::Range>>,
    ) -> axum_range::Ranged<axum_range::KnownSize<Body>> {
        let len = state.data.len() as u64;
        let body = axum_range::KnownSize::sized(state, len);
        let range = range.map(|axum_extra::TypedHeader(range)| range);

        axum_range::Ranged::new(range, body)
    }
}

/// Convenience macro for getting a ranged request Tube
#[macro_export]
macro_rules! tube {
    ($body:expr) => {
        Tube::new_range_request_server($body)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;

    #[tokio::test]
    async fn test_range_request_good() {
        let tb = tube!("happy valentine's day".as_bytes()).await;

        let client = Client::new();
        let response = client.get(tb.url()).send().await.unwrap();

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.bytes().await.unwrap(),
            "happy valentine's day".as_bytes()
        );

        let response = client
            .get(tb.url())
            .header("Range", "bytes=0-5")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 206);
        assert_eq!(response.bytes().await.unwrap(), "happy ".as_bytes());

        let response = client
            .get(tb.url())
            .header("Range", "bytes=18-20")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 206);
        assert_eq!(response.bytes().await.unwrap(), "day".as_bytes());

        let response = client
            .get(tb.url())
            .header("Range", "bytes=18-21")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 416);
        assert_eq!(response.bytes().await.unwrap(), "".as_bytes());

        let response = client
            .get(tb.url())
            .header("Range", "bytes=100-200")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 416);
        assert_eq!(response.bytes().await.unwrap(), "".as_bytes());
    }
}

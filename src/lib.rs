use std::task::{Context, Poll};

use axum::Router;
use axum_range::AsyncSeekStart;
use bytes::BufMut;

use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::oneshot,
};

#[derive(Clone)]
struct ResponseBody {
    data: Vec<u8>,
    seek_position: u64,
}
impl ResponseBody {
    fn new(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            seek_position: 0,
        }
    }
}

impl AsyncRead for ResponseBody {
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

impl AsyncSeekStart for ResponseBody {
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

pub struct Tubetti {
    shutdown_tx: oneshot::Sender<()>,
    addr: String,
}

impl Tubetti {
    pub async fn new_range_request_server(body: &[u8]) -> Self {
        let app = Router::new()
            .route("/", axum::routing::get(serve_ranged_request))
            .with_state(ResponseBody::new(body));
        Self::new(app).await
    }

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

        Self { shutdown_tx, addr }
    }

    pub fn shutdown(self) {
        self.shutdown_tx.send(()).expect("shutdown failed");
    }

    pub fn addr(&self) -> String {
        self.addr.clone()
    }
}

#[macro_export]
macro_rules! tube {
    ($body:expr) => {
        Tubetti::new_range_request_server($body)
    };
}

async fn serve_ranged_request(
    axum::extract::State(state): axum::extract::State<ResponseBody>,
    range: Option<axum_extra::TypedHeader<axum_extra::headers::Range>>,
) -> axum_range::Ranged<axum_range::KnownSize<ResponseBody>> {
    let len = state.data.len() as u64;
    let body = axum_range::KnownSize::sized(state, len);
    let range = range.map(|axum_extra::TypedHeader(range)| range);

    axum_range::Ranged::new(range, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;

    #[tokio::test]
    async fn test_range_request_good() {
        let ts = tube!("happy valentine's day".as_bytes()).await;

        let client = Client::new();
        let response = client.get(ts.addr()).send().await.unwrap();

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.bytes().await.unwrap(),
            "happy valentine's day".as_bytes()
        );

        let response = client
            .get(ts.addr())
            .header("Range", "bytes=0-5")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 206);
        assert_eq!(response.bytes().await.unwrap(), "happy ".as_bytes());

        let response = client
            .get(ts.addr())
            .header("Range", "bytes=18-20")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 206);
        assert_eq!(response.bytes().await.unwrap(), "day".as_bytes());

        let response = client
            .get(ts.addr())
            .header("Range", "bytes=18-21")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 416);
        assert_eq!(response.bytes().await.unwrap(), "".as_bytes());

        let response = client
            .get(ts.addr())
            .header("Range", "bytes=100-200")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 416);
        assert_eq!(response.bytes().await.unwrap(), "".as_bytes());
    }
}

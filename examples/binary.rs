use tubetti::Tube;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now = tokio::time::Instant::now();
    let server_uptime = std::time::Duration::from_secs(20);
    let shutdown_at = now + server_uptime;

    let mut headers = tubetti::axum::http::HeaderMap::new();

    headers.append(
        "Content-Type",
        tubetti::axum::http::HeaderValue::from_static("binary/octet-stream"),
    );

    let body = "some-body-content".as_bytes();
    let port = Some(3000);
    let status = Some(tubetti::axum::http::StatusCode::OK);
    let headers = Some(headers);

    let tube = tubetti::tube!(body, port, status, headers).await?;
    let url = tube.url();

    eprintln!();
    eprintln!(
        "Server running at {url} :: Will shutdown in {} seconds. Try `curl -H 'Range: bytes=0-7' {url}`",
        server_uptime.as_secs(),
    );

    tokio::time::sleep_until(shutdown_at).await;

    tube.shutdown().await?;

    eprintln!(" -> Server shutdown successfully");

    Ok(())
}

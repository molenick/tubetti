use tubetti::Tube;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let now = tokio::time::Instant::now();
    let server_uptime = std::time::Duration::from_secs(20);
    let shutdown_at = now + server_uptime;

    let mut headers = tubetti::axum::http::HeaderMap::new();

    headers.append(
        "Content-Type",
        tubetti::axum::http::HeaderValue::from_static("text/html"),
    );

    let body = "<html><body><h1>Hello World!</h1></body></html>".as_bytes();
    let port = Some(3000);
    let status = Some(tubetti::axum::http::StatusCode::OK);
    let headers = Some(headers);

    let tube = tubetti::tube!(body.into(), port, status, headers).await?;
    let url = tube.url();

    eprintln!();
    eprintln!(
        "Server running at {url} :: Will shutdown in {} seconds. Try opening server URL in browser.",
        server_uptime.as_secs()
    );

    tokio::time::sleep_until(shutdown_at).await;

    tube.shutdown().await?;

    eprintln!(" -> Server shutdown successfully");

    Ok(())
}

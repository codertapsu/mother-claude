//! The spike's HTTP stack without iroh (axum + hyper server and client, a
//! WebSocket client): its release size, subtracted from `mc-spike`'s, is what
//! iroh adds. It serves one request to itself so nothing is optimised away.

use axum::{routing::get, Router};
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::TokioIo;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new().route("/spike/hello", get(|| async { "hello" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let stream = tokio::net::TcpStream::connect(addr).await?;
    let (mut tx, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(conn);
    let resp = tx
        .send_request(
            hyper::Request::get("/spike/hello")
                .header("host", "x")
                .body(Empty::<Bytes>::new())?,
        )
        .await?;
    let body = resp.into_body().collect().await?.to_bytes();
    println!("{}", String::from_utf8_lossy(&body));
    if std::env::args().any(|a| a == "--ws") {
        let _ = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await;
    }
    let v: serde_json::Value = serde_json::json!({ "ok": true });
    println!("{v}");
    Ok(())
}

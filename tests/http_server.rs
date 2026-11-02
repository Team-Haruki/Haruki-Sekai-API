//! The shared server loop (`server::serve_with`) answers like `axum::serve`,
//! closes connections that stay idle past the header-read timeout, never cuts
//! off a response that streams for longer than that timeout, and shuts down
//! gracefully.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::routing::get;
use axum::Router;
use haruki_sekai_api::server::{self, ServerOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const HEADER_TIMEOUT: Duration = Duration::from_millis(300);
const STREAM_CHUNKS: usize = 4;
const STREAM_CHUNK_DELAY: Duration = Duration::from_millis(250);

fn app() -> Router {
    Router::new()
        .route("/hello", get(|| async { "hello" }))
        .route(
            "/slow",
            get(|| async {
                let chunks = futures::stream::unfold(0usize, |i| async move {
                    if i == STREAM_CHUNKS {
                        return None;
                    }
                    tokio::time::sleep(STREAM_CHUNK_DELAY).await;
                    Some((Ok::<_, std::io::Error>(format!("chunk{i};")), i + 1))
                });
                Body::from_stream(chunks)
            }),
        )
}

struct TestServer {
    addr: SocketAddr,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

async fn start() -> anyhow::Result<TestServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (stop, stopped) = oneshot::channel::<()>();
    let options = ServerOptions {
        header_read_timeout: HEADER_TIMEOUT,
        ..ServerOptions::default()
    };
    let task = tokio::spawn(server::serve_with(
        listener,
        app(),
        async move {
            let _ = stopped.await;
        },
        options,
    ));
    Ok(TestServer { addr, stop, task })
}

/// Reads one `Content-Length` response off a raw connection.
async fn read_response(stream: &mut TcpStream) -> anyhow::Result<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        anyhow::ensure!(n > 0, "connection closed mid-response");
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|l| l.strip_prefix("content-length: "))
                .map(|v| v.trim().parse::<usize>())
                .transpose()?
                .unwrap_or(0);
            if buf.len() >= end + 4 + length {
                return Ok(text.into_owned());
            }
        }
    }
}

#[tokio::test]
async fn answers_requests_and_reuses_keep_alive_connections() -> anyhow::Result<()> {
    let server = start().await?;
    let client = reqwest::Client::new();
    let url = format!("http://{}/hello", server.addr);
    for _ in 0..3 {
        let res = client.get(&url).send().await?;
        assert_eq!(res.status(), 200);
        assert_eq!(res.text().await?, "hello");
    }
    let missing = client
        .get(format!("http://{}/missing", server.addr))
        .send()
        .await?;
    assert_eq!(missing.status(), 404);

    // Two requests on one raw connection: keep-alive still works.
    let mut stream = TcpStream::connect(server.addr).await?;
    for _ in 0..2 {
        stream
            .write_all(b"GET /hello HTTP/1.1\r\nHost: test\r\n\r\n")
            .await?;
        let response = read_response(&mut stream).await?;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("hello"), "{response}");
    }

    server.stop.send(()).ok();
    drop(stream);
    tokio::time::timeout(Duration::from_secs(5), server.task).await??;
    Ok(())
}

#[tokio::test]
async fn closes_connections_that_stay_idle() -> anyhow::Result<()> {
    let server = start().await?;

    // Never sends a request.
    // Taken before connecting: the server may start its timer before this
    // task resumes from `connect`.
    let started = Instant::now();
    let mut silent = TcpStream::connect(server.addr).await?;
    let mut buf = [0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(5), silent.read(&mut buf)).await??;
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        n == 0 || response.starts_with("HTTP/1.1 408"),
        "idle connection got {response:?}"
    );
    assert!(started.elapsed() >= HEADER_TIMEOUT);

    // Goes idle after one request.
    let mut idle = TcpStream::connect(server.addr).await?;
    idle.write_all(b"GET /hello HTTP/1.1\r\nHost: test\r\n\r\n")
        .await?;
    read_response(&mut idle).await?;
    let n = tokio::time::timeout(Duration::from_secs(5), idle.read(&mut buf)).await??;
    let response = String::from_utf8_lossy(&buf[..n]);
    assert!(
        n == 0 || response.starts_with("HTTP/1.1 408"),
        "idle keep-alive connection got {response:?}"
    );

    server.stop.send(()).ok();
    tokio::time::timeout(Duration::from_secs(5), server.task).await??;
    Ok(())
}

#[tokio::test]
async fn streams_responses_longer_than_the_header_timeout() -> anyhow::Result<()> {
    let server = start().await?;
    assert!(STREAM_CHUNK_DELAY * STREAM_CHUNKS as u32 > HEADER_TIMEOUT * 2);

    let res = reqwest::get(format!("http://{}/slow", server.addr)).await?;
    assert_eq!(res.status(), 200);
    assert_eq!(res.text().await?, "chunk0;chunk1;chunk2;chunk3;");

    server.stop.send(()).ok();
    tokio::time::timeout(Duration::from_secs(5), server.task).await??;
    Ok(())
}

#[tokio::test]
async fn shutdown_lets_an_in_flight_response_finish() -> anyhow::Result<()> {
    let server = start().await?;
    let addr = server.addr;
    // The headers arrive before the first chunk, so the response is in flight.
    let res = reqwest::get(format!("http://{addr}/slow")).await?;
    server.stop.send(()).ok();

    assert_eq!(res.text().await?, "chunk0;chunk1;chunk2;chunk3;");
    tokio::time::timeout(Duration::from_secs(5), server.task).await??;
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "listener closed after shutdown"
    );
    Ok(())
}

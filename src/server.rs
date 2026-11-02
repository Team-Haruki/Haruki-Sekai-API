//! The HTTP server loop shared by every binary that listens: the API, the
//! master registry and the ingest role.
//!
//! `axum::serve` sets no TCP keepalive and no HTTP/1 timeout. A client that
//! disappears without a FIN or RST (a half-open connection, common on lossy
//! long-distance paths) then keeps its socket, and a file descriptor, until
//! the process exits. Enough of them exhaust the descriptor limit, after which
//! accepts, outbound requests, git and file I/O all fail with `EMFILE`.
//!
//! [`serve`] is `axum::serve` with graceful shutdown plus two guards:
//! - TCP keepalive on every accepted socket, so the kernel drops a peer that
//!   stops answering after about two minutes of silence;
//! - an HTTP/1 header-read timeout, which also bounds how long a keep-alive
//!   connection may sit idle between requests. It never runs while a request
//!   is being handled or a response body is streaming, so long downloads and
//!   streamed bundles are not cut off. There is deliberately no total-request
//!   timeout.
//!
//! The values are constants rather than config: they only bound dead or idle
//! connections and no deployment needs to tune them.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use socket2::{SockRef, TcpKeepalive};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, trace};

/// Idle time before the kernel sends the first keepalive probe.
pub const TCP_KEEPALIVE_TIME: Duration = Duration::from_secs(60);
/// Time between unanswered keepalive probes.
pub const TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// Unanswered probes after which the kernel resets the connection. With the
/// values above, a dead peer is dropped after about 60 + 6 × 10 = 120 s.
pub const TCP_KEEPALIVE_RETRIES: u32 = 6;
/// Time a client has to send a request's headers. On a keep-alive connection
/// the timer starts as soon as the previous response is finished, so this is
/// also the idle timeout between requests. It is longer than the idle timeout
/// of common client pools (90 s in reqwest and Go, 120 s in Caddy), so the
/// client, not the server, normally closes an idle pooled connection and a
/// request never races the server's close.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(180);
/// Pause after an accept error other than a reset/aborted connection, which
/// is usually `EMFILE`/`ENFILE` (out of file descriptors). The same as axum.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_secs(1);

/// Connection limits used by [`serve_with`]; [`serve`] uses the defaults.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub keepalive: TcpKeepalive,
    pub header_read_timeout: Duration,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            keepalive: default_keepalive(),
            header_read_timeout: HEADER_READ_TIMEOUT,
        }
    }
}

/// The keepalive settings applied to accepted sockets.
pub fn default_keepalive() -> TcpKeepalive {
    TcpKeepalive::new()
        .with_time(TCP_KEEPALIVE_TIME)
        .with_interval(TCP_KEEPALIVE_INTERVAL)
        .with_retries(TCP_KEEPALIVE_RETRIES)
}

/// Turns on `SO_KEEPALIVE` with the given idle time, probe interval and probe
/// count.
pub fn set_tcp_keepalive(stream: &TcpStream, keepalive: &TcpKeepalive) -> io::Result<()> {
    SockRef::from(stream).set_tcp_keepalive(keepalive)
}

/// Serves `app` on `listener` until `shutdown` resolves, then stops accepting
/// and waits for open connections to finish, like
/// `axum::serve(..).with_graceful_shutdown(..)`.
pub async fn serve<F>(listener: TcpListener, app: Router, shutdown: F)
where
    F: Future<Output = ()>,
{
    serve_with(listener, app, shutdown, ServerOptions::default()).await;
}

/// [`serve`] with explicit connection limits (tests use short timeouts).
pub async fn serve_with<F>(listener: TcpListener, app: Router, shutdown: F, options: ServerOptions)
where
    F: Future<Output = ()>,
{
    let mut http = http1::Builder::new();
    http.timer(TokioTimer::new())
        .header_read_timeout(options.header_read_timeout);
    let graceful = GracefulShutdown::new();
    let mut accept_failing = false;
    let mut shutdown = std::pin::pin!(shutdown);

    loop {
        let (stream, remote) = tokio::select! {
            accepted = accept(&listener, &mut accept_failing) => accepted,
            () = &mut shutdown => break,
        };
        if let Err(e) = set_tcp_keepalive(&stream, &options.keepalive) {
            debug!("Failed to enable TCP keepalive for {}: {}", remote, e);
        }
        // No HTTP upgrades: no route uses WebSockets or CONNECT.
        let service = TowerToHyperService::new(app.clone());
        let conn = http.serve_connection(TokioIo::new(stream), service);
        let conn = graceful.watch(conn);
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                trace!("Connection from {} ended with an error: {}", remote, e);
            }
        });
    }

    drop(listener);
    graceful.shutdown().await;
}

/// Accepts the next connection. Reset or aborted handshakes are skipped
/// silently; any other error (in practice `EMFILE`) is retried every second
/// and logged once per episode instead of once per attempt.
async fn accept(listener: &TcpListener, failing: &mut bool) -> (TcpStream, SocketAddr) {
    loop {
        match listener.accept().await {
            Ok(accepted) => {
                if std::mem::take(failing) {
                    info!("Accepting connections again");
                }
                return accepted;
            }
            Err(e) if is_connection_error(&e) => {}
            Err(e) => {
                if !std::mem::replace(failing, true) {
                    error!(
                        "Accept failed, retrying every {:?} until it recovers: {}",
                        ACCEPT_ERROR_BACKOFF, e
                    );
                }
                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
            }
        }
    }
}

fn is_connection_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // socket2 has no keepalive interval getter on Windows.
    #[cfg(unix)]
    #[tokio::test]
    async fn keepalive_is_set_on_a_connected_socket() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let client = TcpStream::connect(listener.local_addr()?).await?;
        let (server, _) = listener.accept().await?;

        let before = SockRef::from(&server);
        assert!(
            !before.keepalive()?,
            "accepted sockets start without keepalive"
        );

        set_tcp_keepalive(&server, &default_keepalive())?;
        let sock = SockRef::from(&server);
        assert!(sock.keepalive()?);
        assert_eq!(sock.tcp_keepalive_time()?, TCP_KEEPALIVE_TIME);
        assert_eq!(sock.tcp_keepalive_interval()?, TCP_KEEPALIVE_INTERVAL);
        assert_eq!(sock.tcp_keepalive_retries()?, TCP_KEEPALIVE_RETRIES);
        drop(client);
        Ok(())
    }

    #[test]
    fn reset_handshakes_are_not_accept_failures() {
        assert!(is_connection_error(
            &io::ErrorKind::ConnectionAborted.into()
        ));
        assert!(is_connection_error(&io::ErrorKind::ConnectionReset.into()));
        assert!(!is_connection_error(&io::Error::from_raw_os_error(24)));
    }
}

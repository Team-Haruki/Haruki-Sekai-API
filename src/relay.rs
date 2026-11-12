//! Egress relay: the other end of a `servers.<region>.egress` route.
//!
//! A relay node lends its IP to the account node and nothing else. It accepts
//! `CONNECT host:443` from a caller that presents the relay token, checks the
//! host against an allowlist (the game API hosts), and then copies bytes in
//! both directions. TLS runs end to end between the account node and the game,
//! so the relay never sees a session token, a request, or a response, and it
//! holds no accounts and no keys. It is not a general proxy: no plain HTTP
//! forwarding, no other hosts, no other ports.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::config::EgressRelayConfig;

/// Upper bound for the CONNECT request head.
const MAX_HEAD_BYTES: usize = 8 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
/// Share of the dial deadline one resolved address may use, so an address
/// that hangs does not keep a reachable one from being tried.
const DIAL_PER_ADDRESS: Duration = Duration::from_secs(4);
/// After one side of a tunnel has closed, how long the other side may keep
/// sending before the tunnel is torn down. Without a bound, a peer that
/// stays open but silent holds both sockets and a tunnel slot forever.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
/// Tunnels open at once. The account node keeps a few pooled connections per
/// region, so this is far above real use and only bounds a runaway caller.
const MAX_TUNNELS: usize = 512;
/// The only port a tunnel may target unless an allowlist entry names another.
const DEFAULT_PORT: u16 = 443;

/// A validated relay configuration.
pub struct Relay {
    token: String,
    /// Lowercased `host:port` targets.
    allowed: Vec<String>,
}

impl Relay {
    pub fn new(config: &EgressRelayConfig) -> Result<Self, String> {
        if config.token.is_empty() {
            return Err("egress_relay.token must be set".to_string());
        }
        if config.allow_hosts.is_empty() {
            return Err("egress_relay.allow_hosts must list at least one host".to_string());
        }
        let allowed = config
            .allow_hosts
            .iter()
            .map(|entry| {
                let entry = entry.trim().to_ascii_lowercase();
                if entry.contains(':') {
                    entry
                } else {
                    format!("{entry}:{DEFAULT_PORT}")
                }
            })
            .collect();
        Ok(Self {
            token: config.token.clone(),
            allowed,
        })
    }

    fn authorized(&self, header: Option<&str>) -> bool {
        let Some(encoded) = header.and_then(|value| {
            let (scheme, rest) = value.trim().split_once(' ')?;
            scheme.eq_ignore_ascii_case("basic").then(|| rest.trim())
        }) else {
            return false;
        };
        let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
            return false;
        };
        // `user:password`; only the password (the relay token) matters.
        let password = match decoded.iter().position(|byte| *byte == b':') {
            Some(index) => &decoded[index + 1..],
            None => return false,
        };
        constant_time_eq(password, self.token.as_bytes())
    }

    fn allows(&self, target: &str) -> bool {
        self.allowed.contains(&target.to_ascii_lowercase())
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

/// TCP keepalive on both ends of a tunnel (the settings the API server uses).
/// A peer that vanished without closing (the account node lost power or its
/// link, which is exactly when failover is in play) is dropped by the kernel
/// after about two minutes, and the tunnel and its two sockets are released
/// instead of piling up.
fn enable_keepalive(stream: &TcpStream) {
    if let Err(e) = crate::server::set_tcp_keepalive(stream, &crate::server::default_keepalive()) {
        debug!("Relay could not enable TCP keepalive: {}", e);
    }
}

/// Accept tunnels until `shutdown` resolves.
pub async fn serve(
    listener: TcpListener,
    relay: Arc<Relay>,
    shutdown: impl std::future::Future<Output = ()>,
) {
    let tunnels = Arc::new(tokio::sync::Semaphore::new(MAX_TUNNELS));
    tokio::pin!(shutdown);
    loop {
        let accepted = tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, peer)) => {
                let Ok(permit) = tunnels.clone().try_acquire_owned() else {
                    warn!("Relay at {} tunnels, refusing {}", MAX_TUNNELS, peer);
                    continue;
                };
                enable_keepalive(&stream);
                let relay = relay.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = handle(stream, &relay).await {
                        debug!("Relay connection from {} ended: {}", peer, e);
                    }
                });
            }
            Err(e) => {
                warn!("Relay accept failed: {}", e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    info!("Egress relay stopped accepting connections");
}

async fn reject(stream: &mut TcpStream, status: &str, extra: &str) -> std::io::Result<()> {
    let response =
        format!("HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

/// Read up to the end of the request head. Returns the head and whatever the
/// client already sent after it.
async fn read_head(stream: &mut TcpStream) -> std::io::Result<Option<(String, Vec<u8>)>> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = buffer.split_off(end + 4);
            return Ok(String::from_utf8(buffer).ok().map(|head| (head, rest)));
        }
        if buffer.len() >= MAX_HEAD_BYTES {
            return Ok(None);
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

async fn handle(mut inbound: TcpStream, relay: &Relay) -> std::io::Result<()> {
    let head = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut inbound)).await {
        Ok(Ok(Some(head))) => head,
        Ok(Ok(None)) => return reject(&mut inbound, "400 Bad Request", "").await,
        Ok(Err(e)) => return Err(e),
        Err(_) => return reject(&mut inbound, "408 Request Timeout", "").await,
    };
    let (head, early_bytes) = head;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let (method, target) = (
        request_line.next().unwrap_or_default(),
        request_line.next().unwrap_or_default(),
    );
    let authorization = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("proxy-authorization")
            .then_some(value)
    });
    if !relay.authorized(authorization) {
        return reject(
            &mut inbound,
            "407 Proxy Authentication Required",
            "Proxy-Authenticate: Basic realm=\"haruki-egress\"\r\n",
        )
        .await;
    }
    if method != "CONNECT" {
        return reject(&mut inbound, "405 Method Not Allowed", "Allow: CONNECT\r\n").await;
    }
    if !relay.allows(target) {
        warn!("Relay refused a tunnel to {}", target);
        return reject(&mut inbound, "403 Forbidden", "").await;
    }
    let mut outbound = match tokio::time::timeout(DIAL_TIMEOUT, dial(target)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::TimedOut => {
            warn!("Relay timed out reaching {}", target);
            return reject(&mut inbound, "504 Gateway Timeout", "").await;
        }
        Ok(Err(e)) => {
            warn!("Relay could not reach {}: {}", target, e);
            return reject(&mut inbound, "502 Bad Gateway", "").await;
        }
        Err(_) => {
            warn!("Relay timed out reaching {}", target);
            return reject(&mut inbound, "504 Gateway Timeout", "").await;
        }
    };
    let _ = inbound.set_nodelay(true);
    let _ = outbound.set_nodelay(true);
    enable_keepalive(&outbound);
    inbound
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    if !early_bytes.is_empty() {
        outbound.write_all(&early_bytes).await?;
    }
    pipe(inbound, outbound, DRAIN_TIMEOUT).await;
    Ok(())
}

/// Connect to `target`, giving each resolved address a bounded attempt. A
/// plain `TcpStream::connect` tries addresses one after another with no
/// per-address limit, so one hanging address would use up the whole deadline.
async fn dial(target: &str) -> std::io::Result<TcpStream> {
    let mut last_error = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "the host resolved to no address",
    );
    for address in tokio::net::lookup_host(target).await? {
        match tokio::time::timeout(DIAL_PER_ADDRESS, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last_error = e,
            Err(_) => {
                last_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timed out")
            }
        }
    }
    Err(last_error)
}

/// Copy both directions until both are done. Once one direction reaches EOF
/// (or fails) its write side is shut down and the other direction gets
/// `drain` to finish.
async fn pipe(mut inbound: TcpStream, mut outbound: TcpStream, drain: Duration) {
    let (mut inbound_read, mut inbound_write) = inbound.split();
    let (mut outbound_read, mut outbound_write) = outbound.split();
    let upstream = async {
        let _ = tokio::io::copy(&mut inbound_read, &mut outbound_write).await;
        let _ = outbound_write.shutdown().await;
    };
    let downstream = async {
        let _ = tokio::io::copy(&mut outbound_read, &mut inbound_write).await;
        let _ = inbound_write.shutdown().await;
    };
    tokio::pin!(upstream, downstream);
    tokio::select! {
        _ = &mut upstream => {
            let _ = tokio::time::timeout(drain, &mut downstream).await;
        }
        _ = &mut downstream => {
            let _ = tokio::time::timeout(drain, &mut upstream).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(allow: &[&str]) -> EgressRelayConfig {
        EgressRelayConfig {
            listen: "127.0.0.1:0".to_string(),
            token: "relay-token".to_string(),
            allow_hosts: allow.iter().map(|host| host.to_string()).collect(),
        }
    }

    fn basic(user: &str, password: &str) -> String {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
        )
    }

    /// An HTTP/1.1 target that answers every request with a fixed body.
    async fn spawn_target() -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().fallback(|| async { "through the tunnel" });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (address, task)
    }

    async fn spawn_relay(allow: &[&str]) -> (String, tokio::task::JoinHandle<()>) {
        let relay = Arc::new(Relay::new(&config(allow)).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let task = tokio::spawn(serve(listener, relay, std::future::pending()));
        (address, task)
    }

    /// Send one request head to the relay and return everything it answers
    /// until it closes or `expect` shows up.
    async fn exchange(relay: &str, request: &str, expect: &str) -> String {
        let mut stream = TcpStream::connect(relay).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut received = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
                .await
                .expect("relay answered in time")
                .unwrap();
            if read == 0 {
                break;
            }
            received.extend_from_slice(&chunk[..read]);
            if String::from_utf8_lossy(&received).contains(expect) {
                break;
            }
        }
        String::from_utf8_lossy(&received).into_owned()
    }

    #[test]
    fn config_requires_a_token_and_hosts_and_defaults_the_port() {
        let mut bad = config(&["game.example"]);
        bad.token.clear();
        assert!(Relay::new(&bad).is_err());
        assert!(Relay::new(&config(&[])).is_err());

        let relay = Relay::new(&config(&["Game.Example", "other.example:8443"])).unwrap();
        assert!(relay.allows("game.example:443"));
        assert!(relay.allows("GAME.example:443"));
        assert!(relay.allows("other.example:8443"));
        assert!(!relay.allows("game.example:80"));
        assert!(!relay.allows("other.example:443"));
        assert!(!relay.allows("evil.example:443"));
        assert!(!relay.allows("game.example"));
    }

    #[test]
    fn authorization_checks_only_the_token() {
        let relay = Relay::new(&config(&["game.example"])).unwrap();
        assert!(relay.authorized(Some(&basic("anyone", "relay-token"))));
        assert!(relay.authorized(Some(&basic("", "relay-token"))));
        assert!(relay.authorized(Some(&basic("x", "relay-token").replace("Basic", "basic"))));
        assert!(!relay.authorized(Some(&basic("relay-token", "wrong"))));
        assert!(!relay.authorized(Some(&basic("x", "relay-token-longer"))));
        assert!(!relay.authorized(Some("Bearer relay-token")));
        assert!(!relay.authorized(Some("Basic !!!")));
        assert!(
            !relay.authorized(Some("Basic cmVsYXktdG9rZW4=")),
            "no colon"
        );
        assert!(!relay.authorized(None));
    }

    // The relay's whole job: an authorized CONNECT to an allowed host becomes
    // a byte pipe, and everything else is refused before any dial.
    #[tokio::test]
    async fn tunnels_allowed_targets_and_refuses_everything_else() {
        let (target, target_task) = spawn_target().await;
        let (relay, relay_task) = spawn_relay(&[&target]).await;
        let auth = basic("relay", "relay-token");

        // Bytes sent right behind the CONNECT head must reach the target too.
        let tunnelled = exchange(
            &relay,
            &format!(
                "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: {auth}\r\n\r\n\
                 GET / HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n"
            ),
            "through the tunnel",
        )
        .await;
        assert!(tunnelled.starts_with("HTTP/1.1 200 Connection Established\r\n\r\n"));
        assert!(tunnelled.contains("through the tunnel"));

        let cases = [
            (
                format!("CONNECT {target} HTTP/1.1\r\n\r\n"),
                "407 Proxy Authentication Required",
            ),
            (
                format!(
                    "CONNECT {target} HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
                    basic("relay", "wrong")
                ),
                "407 Proxy Authentication Required",
            ),
            (
                format!("CONNECT 127.0.0.1:1 HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"),
                "403 Forbidden",
            ),
            (
                format!("GET http://{target}/ HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"),
                "405 Method Not Allowed",
            ),
        ];
        for (request, expected) in cases {
            let answer = exchange(&relay, &request, "\r\n\r\n").await;
            assert!(
                answer.starts_with(&format!("HTTP/1.1 {expected}")),
                "{answer}"
            );
        }

        let oversized = format!(
            "CONNECT {target} HTTP/1.1\r\nX: {}",
            "a".repeat(MAX_HEAD_BYTES)
        );
        assert!(exchange(&relay, &oversized, "\r\n\r\n")
            .await
            .starts_with("HTTP/1.1 400 Bad Request"));
        relay_task.abort();
        target_task.abort();
    }

    // A client that closes its side while the target stays open and silent
    // must not hold the tunnel forever: the drain is bounded.
    #[tokio::test]
    async fn a_half_closed_tunnel_is_released_after_the_drain_timeout() {
        let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let silent_address = silent.local_addr().unwrap();
        let client_side = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client_address = client_side.local_addr().unwrap();

        let mut client = TcpStream::connect(client_address).await.unwrap();
        let (inbound, _) = client_side.accept().await.unwrap();
        let outbound = TcpStream::connect(silent_address).await.unwrap();
        // The target accepts and then never reads, writes or closes.
        let (_held_open, _) = silent.accept().await.unwrap();

        let drain = Duration::from_millis(200);
        let started = std::time::Instant::now();
        let tunnel = tokio::spawn(pipe(inbound, outbound, drain));
        client.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), tunnel)
            .await
            .expect("the tunnel ends once the drain timeout has passed")
            .unwrap();
        assert!(started.elapsed() >= drain, "it waited for the drain first");
    }

    #[tokio::test]
    async fn dial_reports_a_refused_target_and_reaches_a_live_one() {
        let dead = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().to_string()
        };
        assert!(dial(&dead).await.is_err());
        let (target, task) = spawn_target().await;
        assert!(dial(&target).await.is_ok());
        task.abort();
    }

    #[tokio::test]
    async fn unreachable_target_is_a_bad_gateway_and_shutdown_stops_the_loop() {
        let dead = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().to_string()
        };
        let relay = Arc::new(Relay::new(&config(&[&dead])).unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve(listener, relay, async {
            let _ = stopped.await;
        }));
        let answer = exchange(
            &address,
            &format!(
                "CONNECT {dead} HTTP/1.1\r\nProxy-Authorization: {}\r\n\r\n",
                basic("relay", "relay-token")
            ),
            "\r\n\r\n",
        )
        .await;
        assert!(answer.starts_with("HTTP/1.1 502 Bad Gateway"), "{answer}");
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("serve returns after shutdown")
            .unwrap();
    }

    // Real TLS through the relay with the account node's own egress client:
    // reqwest must send the token from the route URL on CONNECT and complete
    // the handshake end to end. Needs outbound network, so it is not run in CI:
    // `cargo test --lib relay -- --ignored`.
    #[tokio::test]
    #[ignore = "needs outbound network"]
    async fn real_https_request_goes_through_the_relay() {
        let (relay, relay_task) = spawn_relay(&["example.com"]).await;
        let client = crate::client::SekaiClient::build_egress_http_client(&format!(
            "http://relay:relay-token@{relay}"
        ))
        .unwrap();
        let response = client.get("https://example.com/").send().await.unwrap();
        assert_eq!(response.status(), 200);

        let refused = client.get("https://example.org/").send().await;
        assert!(
            refused.is_err(),
            "a host off the allowlist is not tunnelled"
        );

        let wrong_token = crate::client::SekaiClient::build_egress_http_client(&format!(
            "http://relay:wrong@{relay}"
        ))
        .unwrap();
        assert!(wrong_token
            .get("https://example.com/")
            .send()
            .await
            .is_err());
        relay_task.abort();
    }

    // The account node's side of the route: its egress client must get
    // through the relay with the token carried in the route URL.
    #[tokio::test]
    async fn egress_client_authenticates_with_the_token_in_the_route_url() {
        let (target, target_task) = spawn_target().await;
        let (relay, relay_task) = spawn_relay(&[&target]).await;
        // reqwest tunnels (CONNECT) only for https targets, so drive the
        // handshake it would send: the same Basic credentials it derives from
        // the route URL's userinfo.
        let url = reqwest::Url::parse(&format!("http://relay:relay-token@{relay}")).unwrap();
        let auth = basic(url.username(), url.password().unwrap());
        let answer = exchange(
            &relay,
            &format!("CONNECT {target} HTTP/1.1\r\nProxy-Authorization: {auth}\r\n\r\n"),
            "\r\n\r\n",
        )
        .await;
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(crate::client::SekaiClient::build_egress_http_client(url.as_str()).is_ok());
        relay_task.abort();
        target_task.abort();
    }
}

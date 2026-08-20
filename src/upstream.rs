//! Multi-upstream routing for game API calls.
//!
//! A [`RegionRouter`] fronts one region with an ordered list of targets: the
//! local [`SekaiClient`] and/or remote Haruki Sekai API nodes (reached over the
//! internal network, e.g. Tailscale). Requests try targets in priority order;
//! target-level failures (node down, its accounts broken) fail over to the
//! next target and feed a passive per-target circuit breaker, while game-level
//! outcomes (maintenance, 404s) are returned as-is since another node would
//! see the same thing.
//!
//! Remote targets are driven through the peer node's `POST /internal/sekai-api`
//! endpoint (see `api::internal`), which executes the call on that node's local
//! client only — never through its own router — so forwarding cannot loop.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::{info, warn};

use crate::client::SekaiClient;
use crate::config::{ServerRegion, UpstreamConfig};
use crate::error::AppError;

/// Consecutive target-fault failures before a target's breaker opens.
const BREAKER_FAILURE_THRESHOLD: u32 = 3;
/// How long an open breaker sidelines its target before it is probed again.
const BREAKER_OPEN_COOLDOWN_MS: u64 = 30_000;

/// Request envelope for a peer node's `/internal/sekai-api` endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct InternalApiRequest {
    pub server: String,
    /// "GET" or "POST".
    pub method: String,
    /// Game API path with the `{userId}` placeholder intact; the executing
    /// node substitutes its own account exactly as for a local call.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<JsonValue>,
}

/// Response envelope from `/internal/sekai-api`. Always carried on HTTP 200
/// when the node itself functioned; any non-200 transport status is therefore
/// unambiguously a node-level fault and triggers failover.
#[derive(Debug, Serialize, Deserialize)]
pub struct InternalApiResponse {
    pub ok: bool,
    /// Game-server HTTP status (ok == true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Game response body (ok == true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonValue>,
    /// `AppError::kind()` tag (ok == false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Whether an error indicts the *target* (its node, path, or accounts) rather
/// than the game itself. Target faults fail over to the next target and count
/// toward the breaker; everything else is returned to the caller unchanged,
/// because another node would observe the same outcome (maintenance, 404,
/// caller mistakes).
fn is_target_fault(e: &AppError) -> bool {
    match e {
        AppError::SessionError
        | AppError::CookieExpired
        | AppError::UpgradeRequired
        | AppError::SignatureError
        | AppError::NoAccountError
        | AppError::NoClientAvailable
        | AppError::InvalidHttpStatus(_)
        | AppError::CryptoError(_)
        | AppError::UpstreamData(_)
        | AppError::NetworkError(_)
        | AppError::DatabaseError(_)
        | AppError::RedisError(_)
        | AppError::IoError(_)
        | AppError::Internal(_) => true,
        AppError::UnderMaintenance
        | AppError::InvalidServerRegion(_)
        | AppError::ParseError(_)
        | AppError::AuthError(_)
        | AppError::NotFound(_)
        | AppError::Forbidden(_)
        | AppError::Unknown { .. } => false,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Passive circuit breaker: opens after `BREAKER_FAILURE_THRESHOLD` consecutive
/// target faults, sidelining the target for `BREAKER_OPEN_COOLDOWN_MS`. Expiry
/// is the half-open probe: the next request tries the target again, and one
/// more failure immediately re-arms the cooldown (the counter stays above the
/// threshold until a success resets it).
struct CircuitBreaker {
    consecutive_failures: AtomicU32,
    open_until_ms: AtomicU64,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            open_until_ms: AtomicU64::new(0),
        }
    }

    fn is_open(&self) -> bool {
        now_ms() < self.open_until_ms.load(Ordering::Relaxed)
    }

    fn on_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.open_until_ms.store(0, Ordering::Relaxed);
    }

    /// Returns true when this failure (re-)opened the breaker.
    fn on_failure(&self) -> bool {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if failures >= BREAKER_FAILURE_THRESHOLD {
            self.open_until_ms
                .store(now_ms() + BREAKER_OPEN_COOLDOWN_MS, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

/// A remote Haruki Sekai API node serving one region.
struct RemoteUpstream {
    name: String,
    /// Full URL of the peer's internal endpoint.
    endpoint: String,
    token: String,
    http: reqwest::Client,
}

impl RemoteUpstream {
    async fn call(&self, req: &InternalApiRequest) -> Result<(JsonValue, u16), AppError> {
        let resp = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .json(req)
            .send()
            .await
            .map_err(|e| AppError::NetworkError(format!("upstream {}: {}", self.name, e)))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::NetworkError(format!("upstream {}: {}", self.name, e)))?;
        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            let body = body.chars().take(200).collect::<String>();
            return Err(AppError::NetworkError(format!(
                "upstream {} returned {}: {}",
                self.name, status, body
            )));
        }
        // serde_json (preserve_order) keeps game response key order intact.
        let envelope: InternalApiResponse = serde_json::from_slice(&bytes).map_err(|e| {
            AppError::NetworkError(format!("upstream {}: invalid envelope: {}", self.name, e))
        })?;
        if envelope.ok {
            Ok((
                envelope.data.unwrap_or(JsonValue::Null),
                envelope.status.unwrap_or(200),
            ))
        } else {
            Err(AppError::from_kind(
                envelope.kind.as_deref().unwrap_or(""),
                envelope.status,
                envelope.message.unwrap_or_default(),
            ))
        }
    }
}

enum TargetKind {
    Local(Arc<SekaiClient>),
    Remote(RemoteUpstream),
}

struct Target {
    name: String,
    priority: i32,
    kind: TargetKind,
    breaker: CircuitBreaker,
}

impl Target {
    async fn call(&self, req: &InternalApiRequest) -> Result<(JsonValue, u16), AppError> {
        match &self.kind {
            TargetKind::Local(client) => {
                if req.method == "POST" {
                    let body = req.body.clone().unwrap_or(JsonValue::Null);
                    client
                        .post_game_api_body(&req.path, &body, req.params.as_ref())
                        .await
                } else {
                    client.get_game_api(&req.path, req.params.as_ref()).await
                }
            }
            TargetKind::Remote(remote) => remote.call(req).await,
        }
    }
}

/// Ordered set of targets serving one region's game API calls.
pub struct RegionRouter {
    region: ServerRegion,
    targets: Vec<Target>,
}

impl RegionRouter {
    /// Build a router from the local client (if this node has one for the
    /// region) and the configured remote upstreams. `internal_http` is the
    /// shared node-to-node reqwest client. Returns None when there is nothing
    /// to route to.
    pub fn new(
        region: ServerRegion,
        local: Option<(Arc<SekaiClient>, i32)>,
        upstreams: &[UpstreamConfig],
        internal_http: reqwest::Client,
    ) -> Option<Self> {
        let mut targets = Vec::new();
        // Local goes first so a stable sort keeps it ahead of equal-priority remotes.
        if let Some((client, priority)) = local {
            targets.push(Target {
                name: "local".to_string(),
                priority,
                kind: TargetKind::Local(client),
                breaker: CircuitBreaker::new(),
            });
        }
        for up in upstreams {
            if up.url.is_empty() {
                warn!(
                    "{} Skipping upstream with empty url",
                    region.as_str().to_uppercase()
                );
                continue;
            }
            let name = if up.name.is_empty() {
                up.url.clone()
            } else {
                up.name.clone()
            };
            targets.push(Target {
                name,
                priority: up.priority,
                kind: TargetKind::Remote(RemoteUpstream {
                    name: if up.name.is_empty() {
                        up.url.clone()
                    } else {
                        up.name.clone()
                    },
                    endpoint: format!("{}/internal/sekai-api", up.url.trim_end_matches('/')),
                    token: up.token.clone(),
                    http: internal_http.clone(),
                }),
                breaker: CircuitBreaker::new(),
            });
        }
        if targets.is_empty() {
            return None;
        }
        targets.sort_by_key(|t| t.priority);
        info!(
            "{} Region router: {}",
            region.as_str().to_uppercase(),
            targets
                .iter()
                .map(|t| format!("{}(p{})", t.name, t.priority))
                .collect::<Vec<_>>()
                .join(" -> ")
        );
        Some(Self { region, targets })
    }

    pub async fn get_game_api(
        &self,
        path: &str,
        params: Option<&HashMap<String, String>>,
    ) -> Result<(JsonValue, u16), AppError> {
        let req = InternalApiRequest {
            server: self.region.as_str().to_string(),
            method: "GET".to_string(),
            path: path.to_string(),
            params: params.cloned(),
            body: None,
        };
        self.dispatch(&req).await
    }

    pub async fn post_game_api_body<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        params: Option<&HashMap<String, String>>,
    ) -> Result<(JsonValue, u16), AppError> {
        let body = serde_json::to_value(body)
            .map_err(|e| AppError::ParseError(format!("body serialization: {}", e)))?;
        let req = InternalApiRequest {
            server: self.region.as_str().to_string(),
            method: "POST".to_string(),
            path: path.to_string(),
            params: params.cloned(),
            body: Some(body),
        };
        self.dispatch(&req).await
    }

    /// Try targets in priority order, closed breakers first; targets with open
    /// breakers are appended as a last resort so a total outage still probes
    /// them instead of failing without an attempt. Target faults fail over,
    /// game-level outcomes return immediately.
    async fn dispatch(&self, req: &InternalApiRequest) -> Result<(JsonValue, u16), AppError> {
        let closed = self.targets.iter().filter(|t| !t.breaker.is_open());
        let open = self.targets.iter().filter(|t| t.breaker.is_open());
        let mut last_err = AppError::NoClientAvailable;
        for target in closed.chain(open) {
            match target.call(req).await {
                Ok(result) => {
                    target.breaker.on_success();
                    return Ok(result);
                }
                Err(e) if is_target_fault(&e) => {
                    let opened = target.breaker.on_failure();
                    warn!(
                        "{} Target '{}' failed ({}){}, trying next",
                        self.region.as_str().to_uppercase(),
                        target.name,
                        e,
                        if opened { ", breaker opened" } else { "" }
                    );
                    last_err = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }
}

/// Build the reqwest client used for node-to-node internal calls. No proxy
/// (peers are reached over the internal network directly) and a generous
/// timeout: the executing node may itself spend tens of seconds in the game
/// API retry/relogin state machine.
pub fn build_internal_http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(75))
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::NetworkError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_opens_after_threshold_and_resets_on_success() {
        let b = CircuitBreaker::new();
        assert!(!b.is_open());
        assert!(!b.on_failure());
        assert!(!b.on_failure());
        assert!(!b.is_open(), "below threshold stays closed");
        assert!(b.on_failure(), "third failure opens");
        assert!(b.is_open());
        b.on_success();
        assert!(!b.is_open());
        assert!(!b.on_failure(), "counter reset by success");
    }

    #[test]
    fn breaker_reopens_immediately_once_over_threshold() {
        let b = CircuitBreaker::new();
        for _ in 0..3 {
            b.on_failure();
        }
        // Simulate cooldown expiry (half-open probe window).
        b.open_until_ms.store(0, Ordering::Relaxed);
        assert!(!b.is_open());
        assert!(b.on_failure(), "single failure past threshold re-opens");
        assert!(b.is_open());
    }

    #[test]
    fn target_fault_classification() {
        // Fail over: the target node / its accounts are at fault.
        for e in [
            AppError::SessionError,
            AppError::NoAccountError,
            AppError::NoClientAvailable,
            AppError::NetworkError("x".into()),
            AppError::Internal("x".into()),
        ] {
            assert!(is_target_fault(&e), "{e:?} should fail over");
        }
        // Terminal: every node would see the same outcome.
        for e in [
            AppError::UnderMaintenance,
            AppError::ParseError("x".into()),
            AppError::NotFound("x".into()),
            AppError::Unknown {
                status: 404,
                body: String::new(),
            },
        ] {
            assert!(!is_target_fault(&e), "{e:?} should be terminal");
        }
    }

    #[test]
    fn error_kind_round_trips_through_envelope() {
        for e in [
            AppError::SessionError,
            AppError::UnderMaintenance,
            AppError::NetworkError("boom".into()),
            AppError::Unknown {
                status: 404,
                body: "nf".into(),
            },
        ] {
            let status = match &e {
                AppError::Unknown { status, .. } => Some(*status),
                _ => None,
            };
            let rebuilt = AppError::from_kind(e.kind(), status, "boom".into());
            assert_eq!(rebuilt.kind(), e.kind());
            assert_eq!(is_target_fault(&rebuilt), is_target_fault(&e));
        }
    }

    #[test]
    fn envelope_preserves_key_order() {
        let data: JsonValue =
            serde_json::from_str(r#"{"zebra":1,"alpha":2,"mid":{"b":1,"a":2}}"#).unwrap();
        let env = InternalApiResponse {
            ok: true,
            status: Some(200),
            data: Some(data),
            kind: None,
            message: None,
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: InternalApiResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&parsed.data).unwrap(),
            r#"{"zebra":1,"alpha":2,"mid":{"b":1,"a":2}}"#
        );
    }

    #[test]
    fn router_orders_targets_by_priority_with_local_first_on_ties() {
        let http = reqwest::Client::new();
        let ups = vec![
            UpstreamConfig {
                url: "http://a".into(),
                token: String::new(),
                priority: 10,
                name: "a".into(),
            },
            UpstreamConfig {
                url: "http://b".into(),
                token: String::new(),
                priority: -1,
                name: "b".into(),
            },
        ];
        let router = RegionRouter::new(ServerRegion::Cn, None, &ups, http).unwrap();
        let names: Vec<_> = router.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
    }
}

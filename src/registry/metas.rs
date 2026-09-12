//! music_metas management: the registry periodically pulls the five regional
//! `music_metas*.json` files from their upstream, injects the synthetic
//! "omakase" rows every consumer expects (the same rows Haruki-Cloud used to
//! add on its own), stores each result content-addressed, and serves it with
//! a mutable pointer plus immutable blobs. Consumers stop fetching upstream
//! themselves and read the registry instead.
//!
//! State layout under `<state_dir>/metas/<region>/`: `current.json` (the
//! [`MetasRecord`] pointer) and `<sha256>.json` blobs (current plus the
//! previous one, older blobs are pruned).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tracing::{info, warn};

use super::state::is_hex_digest;
use crate::client::helper::write_file_atomic;
use crate::config::{Config, ServerRegion};
use crate::error::AppError;

/// Community-maintained upstream, updated alongside game releases. Note the
/// `-tc` suffix for TW.
pub const DEFAULT_SOURCES: [(ServerRegion, &str); 5] = [
    (
        ServerRegion::Jp,
        "https://sekai-data.3-3.dev/music_metas.json",
    ),
    (
        ServerRegion::En,
        "https://sekai-data.3-3.dev/music_metas-en.json",
    ),
    (
        ServerRegion::Tw,
        "https://sekai-data.3-3.dev/music_metas-tc.json",
    ),
    (
        ServerRegion::Kr,
        "https://sekai-data.3-3.dev/music_metas-kr.json",
    ),
    (
        ServerRegion::Cn,
        "https://sekai-data.3-3.dev/music_metas-cn.json",
    ),
];

/// Upstream responses larger than this are rejected (a music_metas file is a
/// few MB; this bounds a misbehaving upstream).
const MAX_RESPONSE_BYTES: usize = 64 << 20;
/// Blobs kept per region besides the current one.
const PREVIOUS_BLOBS_KEPT: usize = 1;

/// The mutable pointer for one region's music_metas.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetasRecord {
    pub region: String,
    /// SHA-256 of the served (processed) bytes; the blob name and ETag.
    pub sha256: String,
    pub size: u64,
    /// Last successful upstream check (200 or 304).
    pub fetched_at: String,
    /// When the served bytes last changed.
    pub changed_at: String,
    pub source_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_last_modified: Option<String>,
    pub omakase_injected: bool,
    pub rows: usize,
}

/// Outcome of one refresh.
#[derive(Debug, Clone)]
pub struct MetasRefresh {
    pub record: MetasRecord,
    /// The served bytes changed (new blob written).
    pub changed: bool,
    /// Upstream answered 304.
    pub not_modified: bool,
}

pub struct MusicMetasManager {
    dir: PathBuf,
    http: reqwest::Client,
    sources: Vec<(ServerRegion, String)>,
    inject_omakase: bool,
    locks: HashMap<ServerRegion, tokio::sync::Mutex<()>>,
}

impl MusicMetasManager {
    pub fn new(config: &Config) -> Result<Self, AppError> {
        let settings = &config.registry.music_metas;
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent(format!(
                "Haruki-Master-Registry/{}",
                env!("CARGO_PKG_VERSION")
            ));
        // `music_metas.proxy` wins over the node-wide setting when present,
        // including an empty string, which forces this fetch direct. Keeps a
        // proxy that exists only for git from becoming a dependency of the
        // metas feed.
        // Three states, matching `GitHelper`:
        //   `music_metas.proxy` absent + top-level empty -> leave the builder
        //       alone (reqwest may still honour HTTP_PROXY from the env; this is
        //       the historical behaviour and is left untouched),
        //   resolved to an empty string by an explicit override -> `no_proxy()`,
        //       because reqwest reads HTTP_PROXY/HTTPS_PROXY by default and
        //       "not configured" is therefore not the same as "direct",
        //   resolved to a URL -> use it.
        match settings.proxy.as_deref() {
            Some("") => builder = builder.no_proxy(),
            Some(proxy) => {
                builder = builder.proxy(
                    reqwest::Proxy::all(proxy)
                        .map_err(|e| AppError::NetworkError(format!("proxy: {e}")))?,
                );
            }
            None if !config.proxy.is_empty() => {
                builder = builder.proxy(
                    reqwest::Proxy::all(&config.proxy)
                        .map_err(|e| AppError::NetworkError(format!("proxy: {e}")))?,
                );
            }
            None => {}
        }
        let http = builder
            .build()
            .map_err(|e| AppError::NetworkError(format!("http client: {e}")))?;
        let mut sources: Vec<(ServerRegion, String)> = DEFAULT_SOURCES
            .iter()
            .map(|(region, url)| {
                let url = settings
                    .sources
                    .get(region)
                    .cloned()
                    .unwrap_or_else(|| url.to_string());
                (*region, url)
            })
            .filter(|(_, url)| !url.trim().is_empty())
            .collect();
        sources.sort_by_key(|(region, _)| *region);
        let locks = sources
            .iter()
            .map(|(region, _)| (*region, tokio::sync::Mutex::new(())))
            .collect();
        Ok(Self {
            dir: PathBuf::from(&config.registry.state_dir).join("metas"),
            http,
            sources,
            inject_omakase: settings.inject_omakase,
            locks,
        })
    }

    pub fn regions(&self) -> Vec<ServerRegion> {
        self.sources.iter().map(|(region, _)| *region).collect()
    }

    pub fn source_url(&self, region: ServerRegion) -> Option<&str> {
        self.sources
            .iter()
            .find(|(r, _)| *r == region)
            .map(|(_, url)| url.as_str())
    }

    fn region_dir(&self, region: ServerRegion) -> PathBuf {
        self.dir.join(region.as_str())
    }

    fn record_path(&self, region: ServerRegion) -> PathBuf {
        self.region_dir(region).join("current.json")
    }

    /// Path of a content blob; `None` for anything that is not a digest.
    pub fn blob_path(&self, region: ServerRegion, sha256: &str) -> Option<PathBuf> {
        is_hex_digest(sha256).then(|| self.region_dir(region).join(format!("{sha256}.json")))
    }

    pub async fn current(&self, region: ServerRegion) -> Result<Option<MetasRecord>, AppError> {
        match tokio::fs::read(self.record_path(region)).await {
            Ok(data) => serde_json::from_slice(&data)
                .map(Some)
                .map_err(|e| AppError::ParseError(format!("metas record: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Conditional fetch of one region: 304 refreshes the pointer's
    /// `fetchedAt`; 200 is parsed, processed, stored as a new blob when its
    /// digest differs, and becomes the pointer.
    pub async fn refresh(&self, region: ServerRegion) -> Result<MetasRefresh, AppError> {
        let Some(url) = self.source_url(region).map(str::to_string) else {
            return Err(AppError::NotFound(format!(
                "no music_metas source for region {}",
                region.as_str()
            )));
        };
        let _guard = match self.locks.get(&region) {
            Some(lock) => lock.lock().await,
            None => return Err(AppError::InvalidServerRegion(region.as_str().to_string())),
        };
        let existing = self.current(region).await?;
        let mut req = self.http.get(&url);
        if let Some(prev) = existing.as_ref().filter(|r| r.source_url == url) {
            if let Some(etag) = &prev.source_etag {
                req = req.header("if-none-match", etag);
            }
            if let Some(modified) = &prev.source_last_modified {
                req = req.header("if-modified-since", modified);
            }
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::NetworkError(format!("music_metas {}: {e}", region.as_str())))?;
        let now = now_rfc3339();
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            let Some(mut record) = existing else {
                return Err(AppError::UpstreamData(format!(
                    "music_metas {}: 304 without a stored copy",
                    region.as_str()
                )));
            };
            record.fetched_at = now;
            self.write_record(region, &record).await?;
            return Ok(MetasRefresh {
                record,
                changed: false,
                not_modified: true,
            });
        }
        if !resp.status().is_success() {
            return Err(AppError::NetworkError(format!(
                "music_metas {} upstream returned {}",
                region.as_str(),
                resp.status()
            )));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_RESPONSE_BYTES {
                return Err(AppError::UpstreamData(format!(
                    "music_metas {} upstream body too large ({} bytes)",
                    region.as_str(),
                    len
                )));
            }
        }
        let source_etag = header_string(&resp, "etag");
        let source_last_modified = header_string(&resp, "last-modified");
        let body = read_bounded_body(resp, region).await?;
        let inject = self.inject_omakase;
        let (processed, rows, injected) =
            tokio::task::spawn_blocking(move || prepare_music_metas(&body, inject))
                .await
                .map_err(|e| AppError::Internal(format!("metas task: {e}")))??;
        let sha256 = sha256_hex(&processed);
        let changed = existing
            .as_ref()
            .map(|r| r.sha256 != sha256)
            .unwrap_or(true);
        let record = MetasRecord {
            region: region.as_str().to_string(),
            sha256: sha256.clone(),
            size: processed.len() as u64,
            fetched_at: now.clone(),
            changed_at: if changed {
                now
            } else {
                existing
                    .as_ref()
                    .map(|r| r.changed_at.clone())
                    .unwrap_or(now)
            },
            source_url: url,
            source_etag,
            source_last_modified,
            omakase_injected: injected,
            rows,
        };
        tokio::fs::create_dir_all(self.region_dir(region)).await?;
        let blob = self
            .blob_path(region, &sha256)
            .expect("sha256_hex yields a digest");
        if !tokio::fs::try_exists(&blob).await.unwrap_or(false) {
            write_file_atomic(&blob, &processed).await?;
        }
        self.write_record(region, &record).await?;
        if changed {
            info!(
                "{} music_metas updated: {} rows, {} bytes, sha256 {}",
                region.as_str().to_uppercase(),
                rows,
                processed.len(),
                &sha256[..12]
            );
            self.prune_blobs(
                region,
                &sha256,
                existing.as_ref().map(|r| r.sha256.as_str()),
            )
            .await;
        }
        Ok(MetasRefresh {
            record,
            changed,
            not_modified: false,
        })
    }

    /// Refresh every configured region, logging failures; used by the tick.
    pub async fn refresh_all(&self) {
        for region in self.regions() {
            match self.refresh(region).await {
                Ok(outcome) if outcome.changed => {}
                Ok(_) => {
                    tracing::debug!("{} music_metas unchanged", region.as_str().to_uppercase())
                }
                Err(e) => warn!(
                    "{} music_metas refresh failed: {}",
                    region.as_str().to_uppercase(),
                    e
                ),
            }
        }
    }

    async fn write_record(
        &self,
        region: ServerRegion,
        record: &MetasRecord,
    ) -> Result<(), AppError> {
        tokio::fs::create_dir_all(self.region_dir(region)).await?;
        let json = serde_json::to_vec_pretty(record)
            .map_err(|e| AppError::ParseError(format!("metas record: {e}")))?;
        write_file_atomic(&self.record_path(region), &json).await?;
        Ok(())
    }

    /// Keep the current blob and the previous one; remove older blobs.
    async fn prune_blobs(&self, region: ServerRegion, current: &str, previous: Option<&str>) {
        let keep: Vec<&str> = std::iter::once(current)
            .chain(previous.into_iter().take(PREVIOUS_BLOBS_KEPT))
            .collect();
        let Ok(mut rd) = tokio::fs::read_dir(self.region_dir(region)).await else {
            return;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            if is_hex_digest(stem) && !keep.contains(&stem) {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

/// Read the body chunk by chunk, aborting as soon as it exceeds
/// `MAX_RESPONSE_BYTES` instead of buffering an oversized upstream reply.
async fn read_bounded_body(
    mut resp: reqwest::Response,
    region: ServerRegion,
) -> Result<Vec<u8>, AppError> {
    let mut body = Vec::with_capacity(
        resp.content_length()
            .unwrap_or(0)
            .min(MAX_RESPONSE_BYTES as u64) as usize,
    );
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| AppError::NetworkError(format!("music_metas {}: {e}", region.as_str())))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(AppError::UpstreamData(format!(
                "music_metas {} upstream body exceeds {} bytes",
                region.as_str(),
                MAX_RESPONSE_BYTES
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn header_string(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|v| !v.is_empty())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(data))
}

/// Parse an upstream music_metas payload (a JSON array of row objects),
/// optionally inject the omakase rows, and return the served bytes, the row
/// count and whether rows were injected. The upstream rows are never
/// re-serialized: injected rows are spliced onto the original bytes, so
/// every real row is served exactly as upstream wrote it.
pub fn prepare_music_metas(body: &[u8], inject: bool) -> Result<(Vec<u8>, usize, bool), AppError> {
    let mut rows: Vec<JsonMap<String, JsonValue>> = serde_json::from_slice(body)
        .map_err(|e| AppError::UpstreamData(format!("music_metas is not a row array: {e}")))?;
    if rows.is_empty() {
        return Err(AppError::UpstreamData(
            "music_metas payload has no rows".to_string(),
        ));
    }
    if !inject {
        return Ok((body.to_vec(), rows.len(), false));
    }
    let existing = rows.len();
    if !inject_omakase_rows(&mut rows) {
        return Ok((body.to_vec(), existing, false));
    }
    let appended = serde_json::to_vec(&rows[existing..])
        .map_err(|e| AppError::ParseError(format!("music_metas: {e}")))?;
    // `body` is a non-empty array, so it ends with `]` after optional
    // whitespace; `appended` is `[...]` of the new rows.
    let close = body
        .iter()
        .rposition(|b| *b == b']')
        .ok_or_else(|| AppError::UpstreamData("music_metas array is unterminated".to_string()))?;
    let mut processed = Vec::with_capacity(close + appended.len() + 2);
    processed.extend_from_slice(&body[..close]);
    processed.push(b',');
    processed.extend_from_slice(&appended[1..appended.len() - 1]);
    processed.push(b']');
    Ok((processed, rows.len(), true))
}

const OMAKASE_MUSIC_ID: i64 = 10000;
const OMAKASE_DIFFICULTIES: [&str; 3] = ["master", "expert", "hard"];
const AVERAGED_FIELDS: [&str; 7] = [
    "music_time",
    "event_rate",
    "base_score",
    "base_score_auto",
    "fever_score",
    "fever_end_time",
    "tap_count",
];
const AVERAGED_SLICES: [&str; 3] = ["skill_score_solo", "skill_score_auto", "skill_score_multi"];
/// Fields whose average is truncated to an integer (as the original did).
const TRUNCATED_FIELDS: [&str; 2] = ["event_rate", "tap_count"];

/// Ensure the payload carries a synthetic "omakase" entry (music_id 10000)
/// for master, expert and hard: every numeric field is the average over all
/// real rows of those three difficulties. Returns false when the entry already
/// exists or there is nothing to average from. Ported from Haruki-Cloud's
/// `injectOmakaseRows` so the served file matches what consumers received
/// before the registry took the feed over.
pub fn inject_omakase_rows(rows: &mut Vec<JsonMap<String, JsonValue>>) -> bool {
    if rows
        .iter()
        .any(|row| row.get("music_id").and_then(JsonValue::as_f64) == Some(OMAKASE_MUSIC_ID as f64))
    {
        return false;
    }
    let mut sums: HashMap<&str, f64> = AVERAGED_FIELDS.iter().map(|k| (*k, 0.0)).collect();
    let mut slice_sums: HashMap<&str, [f64; 6]> =
        AVERAGED_SLICES.iter().map(|k| (*k, [0.0; 6])).collect();
    let mut count = 0.0f64;
    for row in rows.iter() {
        let difficulty = row
            .get("difficulty")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        if !OMAKASE_DIFFICULTIES.contains(&difficulty) {
            continue;
        }
        count += 1.0;
        for key in AVERAGED_FIELDS {
            if let Some(v) = row.get(key).and_then(JsonValue::as_f64) {
                *sums.get_mut(key).expect("field key") += v;
            }
        }
        for key in AVERAGED_SLICES {
            if let Some(values) = row.get(key).and_then(JsonValue::as_array) {
                let target = slice_sums.get_mut(key).expect("slice key");
                for (i, raw) in values.iter().enumerate().take(6) {
                    if let Some(v) = raw.as_f64() {
                        target[i] += v;
                    }
                }
            }
        }
    }
    if count == 0.0 {
        return false;
    }
    let number = |v: f64| -> JsonValue {
        serde_json::Number::from_f64(v)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null)
    };
    let mut averaged: JsonMap<String, JsonValue> = JsonMap::new();
    for key in AVERAGED_FIELDS {
        let mean = sums[key] / count;
        averaged.insert(
            key.to_string(),
            if TRUNCATED_FIELDS.contains(&key) {
                JsonValue::from(mean.trunc() as i64)
            } else {
                number(mean)
            },
        );
    }
    for key in AVERAGED_SLICES {
        averaged.insert(
            key.to_string(),
            JsonValue::Array(slice_sums[key].iter().map(|v| number(v / count)).collect()),
        );
    }
    for difficulty in OMAKASE_DIFFICULTIES {
        let mut row = JsonMap::new();
        row.insert("music_id".to_string(), JsonValue::from(OMAKASE_MUSIC_ID));
        row.insert(
            "difficulty".to_string(),
            JsonValue::String(difficulty.to_string()),
        );
        for (key, value) in &averaged {
            row.insert(key.clone(), value.clone());
        }
        rows.push(row);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: i64, difficulty: &str, base: f64) -> JsonValue {
        json!({
            "music_id": id, "difficulty": difficulty, "music_time": base, "event_rate": base * 10.0 + 0.7,
            "base_score": base, "base_score_auto": base / 2.0, "fever_score": 1.0, "fever_end_time": 2.0,
            "tap_count": base * 100.0 + 0.9, "skill_score_solo": [1, 2, 3, 4, 5, 6],
            "skill_score_auto": [1, 1, 1, 1, 1, 1], "skill_score_multi": [2, 2, 2, 2, 2, 2, 99]
        })
    }

    #[test]
    fn injects_averaged_omakase_rows_once() {
        let payload = json!([
            row(1, "master", 100.0),
            row(1, "expert", 50.0),
            row(2, "easy", 999.0)
        ]);
        let (processed, rows, injected) =
            prepare_music_metas(&serde_json::to_vec(&payload).unwrap(), true).unwrap();
        assert!(injected);
        assert_eq!(rows, 6);
        let out: Vec<JsonValue> = serde_json::from_slice(&processed).unwrap();
        // Original rows are served byte for byte: the upstream prefix is intact.
        let upstream = serde_json::to_vec(&payload).unwrap();
        assert!(processed.starts_with(&upstream[..upstream.len() - 1]));
        assert_eq!(out[0], payload[0]);
        let omakase: Vec<&JsonValue> = out
            .iter()
            .filter(|r| r["music_id"] == json!(10000))
            .collect();
        assert_eq!(omakase.len(), 3);
        assert_eq!(omakase[0]["difficulty"], "master");
        assert_eq!(omakase[2]["difficulty"], "hard");
        // easy rows are excluded from the average.
        assert_eq!(omakase[0]["music_time"], json!(75.0));
        assert_eq!(omakase[0]["base_score_auto"], json!(37.5));
        // event_rate/tap_count are truncated: (1000.7 + 500.7) / 2 = 750.7 -> 750.
        assert_eq!(omakase[0]["event_rate"], json!(750));
        assert_eq!(omakase[0]["tap_count"], json!(7500));
        assert_eq!(
            omakase[0]["skill_score_solo"],
            json!([1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
        );
        // Only the first six slice entries count.
        assert_eq!(omakase[0]["skill_score_multi"][5], json!(2.0));
        assert_eq!(omakase[0]["skill_score_multi"].as_array().unwrap().len(), 6);

        // Idempotent: a payload that already has omakase rows is served as is.
        let (again, rows2, injected2) = prepare_music_metas(&processed, true).unwrap();
        assert!(!injected2);
        assert_eq!(rows2, 6);
        assert_eq!(again, processed);
        // No qualifying difficulty: nothing to average, bytes untouched.
        let easy = serde_json::to_vec(&json!([row(3, "easy", 1.0)])).unwrap();
        let (same, _, injected3) = prepare_music_metas(&easy, true).unwrap();
        assert!(!injected3);
        assert_eq!(same, easy);
        // Injection disabled keeps upstream bytes.
        let raw = serde_json::to_vec(&payload).unwrap();
        let (kept, _, injected4) = prepare_music_metas(&raw, false).unwrap();
        assert!(!injected4);
        assert_eq!(kept, raw);
        assert!(prepare_music_metas(b"{}", true).is_err());
        assert!(prepare_music_metas(b"[]", true).is_err());
        assert!(prepare_music_metas(b"nope", true).is_err());
    }

    #[derive(Clone)]
    struct Upstream {
        body: std::sync::Arc<parking_lot::Mutex<Vec<u8>>>,
        etag: std::sync::Arc<parking_lot::Mutex<String>>,
        hits: std::sync::Arc<parking_lot::Mutex<Vec<Option<String>>>>,
    }

    async fn upstream_handler(
        axum::extract::State(up): axum::extract::State<Upstream>,
        headers: axum::http::HeaderMap,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        let sent = headers
            .get("if-none-match")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        up.hits.lock().push(sent.clone());
        let etag = up.etag.lock().clone();
        if sent.as_deref() == Some(etag.as_str()) {
            return axum::http::StatusCode::NOT_MODIFIED.into_response();
        }
        (
            axum::http::StatusCode::OK,
            [
                ("content-type", "application/json".to_string()),
                ("etag", etag),
                ("last-modified", "Mon, 07 Sep 2026 04:35:07 GMT".to_string()),
            ],
            up.body.lock().clone(),
        )
            .into_response()
    }

    #[tokio::test]
    async fn refresh_fetches_conditionally_and_prunes_blobs() {
        let up = Upstream {
            body: std::sync::Arc::new(parking_lot::Mutex::new(
                serde_json::to_vec(&json!([row(1, "master", 10.0)])).unwrap(),
            )),
            etag: std::sync::Arc::new(parking_lot::Mutex::new("\"v1\"".to_string())),
            hits: std::sync::Arc::new(parking_lot::Mutex::new(Vec::new())),
        };
        let app = axum::Router::new()
            .fallback(axum::routing::any(upstream_handler))
            .with_state(up.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let root = std::env::temp_dir().join(format!("haruki_metas_{}", uuid::Uuid::new_v4()));
        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        config.registry.state_dir = root.to_string_lossy().into_owned();
        config
            .registry
            .music_metas
            .sources
            .insert(ServerRegion::Jp, format!("http://{addr}/music_metas.json"));
        // Other regions keep their defaults but are not touched by this test.
        let manager = MusicMetasManager::new(&config).unwrap();
        assert_eq!(manager.regions().len(), 5);
        assert!(manager.current(ServerRegion::Jp).await.unwrap().is_none());

        let first = manager.refresh(ServerRegion::Jp).await.unwrap();
        assert!(first.changed && !first.not_modified);
        assert!(first.record.omakase_injected);
        assert_eq!(first.record.rows, 4);
        assert_eq!(first.record.source_etag.as_deref(), Some("\"v1\""));
        let blob1 = manager
            .blob_path(ServerRegion::Jp, &first.record.sha256)
            .unwrap();
        assert!(blob1.exists());
        assert_eq!(std::fs::metadata(&blob1).unwrap().len(), first.record.size);

        // Unchanged upstream: conditional request, 304, pointer refreshed.
        let second = manager.refresh(ServerRegion::Jp).await.unwrap();
        assert!(!second.changed && second.not_modified);
        assert_eq!(second.record.sha256, first.record.sha256);
        assert_eq!(up.hits.lock().last().unwrap().as_deref(), Some("\"v1\""));

        // New upstream content: new blob, old one kept as the previous.
        *up.body.lock() = serde_json::to_vec(&json!([row(1, "master", 20.0)])).unwrap();
        *up.etag.lock() = "\"v2\"".to_string();
        let third = manager.refresh(ServerRegion::Jp).await.unwrap();
        assert!(third.changed);
        assert_ne!(third.record.sha256, first.record.sha256);
        assert!(!third.record.changed_at.is_empty());
        assert!(blob1.exists());
        // A third version prunes the first blob.
        *up.body.lock() = serde_json::to_vec(&json!([row(1, "master", 30.0)])).unwrap();
        *up.etag.lock() = "\"v3\"".to_string();
        let fourth = manager.refresh(ServerRegion::Jp).await.unwrap();
        assert!(fourth.changed);
        assert!(!blob1.exists());
        assert!(manager
            .blob_path(ServerRegion::Jp, &third.record.sha256)
            .unwrap()
            .exists());
        assert!(manager.blob_path(ServerRegion::Jp, "../x").is_none());

        // Upstream failure leaves the pointer untouched.
        server.abort();
        assert!(manager.refresh(ServerRegion::Jp).await.is_err());
        assert_eq!(
            manager
                .current(ServerRegion::Jp)
                .await
                .unwrap()
                .unwrap()
                .sha256,
            fourth.record.sha256
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

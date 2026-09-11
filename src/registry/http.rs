//! Registry HTTP surface.
//!
//! Reads (open on the internal network):
//! - `GET /health`
//! - `GET /v1/master/{region}/current`            latest manifest
//! - `GET /v1/master/{region}/history?limit=N`    publish history, newest first
//! - `GET /v1/master/{region}/files/{name}`       one master file
//! - `GET /v1/master/{region}/bundle`             tar of the master directory
//! - `GET /v1/app/{region}`                       `{appVersion, appHash}` — the
//!   shape the SekaiAPI AppHash updater's `url` source consumes
//!
//! Mutations (require `registry.token`; disabled when it is empty):
//! - `PUT /v1/app/{region}` / `DELETE /v1/app/{region}`  app-identity override
//! - `POST /v1/master/{region}/refresh`            pull from owner now, then publish
//! - `POST /v1/master/{region}/publish`            re-scan the directory and publish
//! - `POST /internal/master-updated`               owner webhook (same shape a
//!   SekaiAPI peer accepts, so an owner's `master_sync.notify` can point here)

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tracing::{error, info};

use super::Registry;
use crate::api::internal::{build_master_tar, MasterUpdatedNotice};
use crate::config::ServerRegion;
use crate::error::AppError;
use crate::updater::apphash::AppInfo;
use crate::updater::master::is_safe_path_component;

type Shared = Arc<Registry>;

pub fn router(registry: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/master/{region}/current", get(current))
        .route("/v1/master/{region}/history", get(history))
        .route("/v1/master/{region}/files/{name}", get(file))
        .route("/v1/master/{region}/bundle", get(bundle))
        .route("/v1/master/{region}/refresh", post(refresh))
        .route("/v1/master/{region}/publish", post(publish))
        .route(
            "/v1/app/{region}",
            get(app_identity)
                .put(set_app_identity)
                .delete(clear_app_identity),
        )
        .route("/internal/master-updated", post(master_updated))
        .with_state(registry)
}

fn parse_region(region: &str) -> Result<ServerRegion, AppError> {
    region
        .parse()
        .map_err(|_| AppError::InvalidServerRegion(region.to_string()))
}

/// Gate a mutating endpoint on `registry.token`: 404 when no token is
/// configured (the endpoint does not exist), 401 on a wrong one.
fn check_token(registry: &Registry, headers: &HeaderMap) -> Result<(), Box<Response>> {
    let expected = &registry.config.registry.token;
    if expected.is_empty() {
        return Err(Box::new(StatusCode::NOT_FOUND.into_response()));
    }
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if presented != Some(expected.as_str()) {
        return Err(Box::new(StatusCode::UNAUTHORIZED.into_response()));
    }
    Ok(())
}

fn json<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_string(value) {
        Ok(body) => (StatusCode::OK, [("content-type", "application/json")], body).into_response(),
        Err(e) => AppError::ParseError(e.to_string()).into_response(),
    }
}

async fn health(State(registry): State<Shared>) -> Response {
    let mut regions = serde_json::Map::new();
    for region in registry.regions() {
        let current = registry.state.current(region).await.ok().flatten();
        regions.insert(
            region.as_str().to_string(),
            serde_json::json!({
                "dataVersion": current.as_ref().map(|m| m.data_version.clone()),
                "publishedAt": current.as_ref().map(|m| m.generated_at.clone()),
                "synced": registry.syncers.contains_key(&region),
            }),
        );
    }
    json(&serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "regions": regions,
    }))
}

async fn current(State(registry): State<Shared>, Path(region): Path<String>) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.state.current(region).await {
        Ok(Some(manifest)) => json(&manifest),
        Ok(None) => {
            AppError::NotFound(format!("region {} has not been published", region.as_str()))
                .into_response()
        }
        Err(e) => e.into_response(),
    }
}

#[derive(Deserialize)]
struct HistoryQuery {
    #[serde(default = "default_history_limit")]
    limit: usize,
}

fn default_history_limit() -> usize {
    20
}

async fn history(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry
        .state
        .history(region, query.limit.clamp(1, 1000))
        .await
    {
        Ok(records) => json(&records),
        Err(e) => e.into_response(),
    }
}

async fn file(
    State(registry): State<Shared>,
    Path((region, name)): Path<(String, String)>,
) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !is_safe_path_component(&name) || !name.ends_with(".json") || name.starts_with('.') {
        return AppError::NotFound(format!("no master file {:?}", name)).into_response();
    }
    let (master_dir, _) = match registry.region_paths(region) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let path = std::path::Path::new(&master_dir).join(&name);
    match tokio::fs::File::open(&path).await {
        Ok(file) => {
            let stream = tokio_util::io::ReaderStream::new(file);
            (
                StatusCode::OK,
                [("content-type", "application/json")],
                axum::body::Body::from_stream(stream),
            )
                .into_response()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            AppError::NotFound(format!("no master file {:?}", name)).into_response()
        }
        Err(e) => AppError::IoError(e.to_string()).into_response(),
    }
}

async fn bundle(State(registry): State<Shared>, Path(region): Path<String>) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let (master_dir, version_path) = match registry.region_paths(region) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let tmp_path = std::env::temp_dir().join(format!(
        "haruki-registry-bundle-{}-{}.tar",
        region.as_str(),
        uuid::Uuid::new_v4()
    ));
    let build_path = tmp_path.clone();
    let built = tokio::task::spawn_blocking(move || {
        build_master_tar(&master_dir, &version_path, &build_path)
    })
    .await;
    let built = match built {
        Ok(r) => r,
        Err(e) => Err(AppError::Internal(format!("bundle task: {e}"))),
    };
    if let Err(e) = built {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return e.into_response();
    }
    match tokio::fs::File::open(&tmp_path).await {
        Ok(file) => {
            // Unlink now; the open fd keeps the bytes until streaming ends.
            let _ = tokio::fs::remove_file(&tmp_path).await;
            (
                StatusCode::OK,
                [("content-type", "application/x-tar")],
                axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
            )
                .into_response()
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            AppError::IoError(e.to_string()).into_response()
        }
    }
}

async fn app_identity(State(registry): State<Shared>, Path(region): Path<String>) -> Response {
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.app_identity(region).await {
        Ok(info) => json(&info),
        Err(e) => e.into_response(),
    }
}

/// Parse a JSON body after the token check has passed, so an unauthorized
/// or disabled endpoint never reports body errors.
fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, AppError> {
    serde_json::from_slice(body).map_err(|e| AppError::ParseError(format!("request body: {e}")))
}

async fn set_app_identity(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let info: AppInfo = match parse_body(&body) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    if info.app_version.trim().is_empty() && info.app_hash.trim().is_empty() {
        return AppError::ParseError("appVersion or appHash is required".to_string())
            .into_response();
    }
    match registry.state.set_app_identity(region, &info).await {
        Ok(()) => {
            info!(
                "{} App identity override set: appVersion={} appHash={}",
                region.as_str().to_uppercase(),
                info.app_version,
                info.app_hash.chars().take(16).collect::<String>()
            );
            json(&info)
        }
        Err(e) => e.into_response(),
    }
}

async fn clear_app_identity(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.state.clear_app_identity(region).await {
        Ok(removed) => json(&serde_json::json!({ "removed": removed })),
        Err(e) => e.into_response(),
    }
}

async fn refresh(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !registry.syncers.contains_key(&region) {
        return AppError::NotFound(format!(
            "no master sync configured for region {}",
            region.as_str()
        ))
        .into_response();
    }
    spawn_refresh(registry, region, "manual refresh");
    json(&serde_json::json!({ "triggered": true }))
}

async fn publish(
    State(registry): State<Shared>,
    Path(region): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let region = match parse_region(&region) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match registry.publish(region).await {
        Ok((manifest, changed)) => json(&serde_json::json!({
            "changed": changed,
            "dataVersion": manifest.data_version,
            "files": manifest.files.len(),
        })),
        Err(e) => e.into_response(),
    }
}

async fn master_updated(
    State(registry): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(resp) = check_token(&registry, &headers) {
        return *resp;
    }
    let notice: MasterUpdatedNotice = match parse_body(&body) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let region = match parse_region(&notice.server) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    if !registry.syncers.contains_key(&region) {
        return AppError::NotFound(format!(
            "no master sync configured for region {}",
            region.as_str()
        ))
        .into_response();
    }
    info!(
        "{} Owner webhook received (dataVersion {:?}), starting sync",
        region.as_str().to_uppercase(),
        notice.data_version
    );
    spawn_refresh(registry, region, "webhook");
    json(&serde_json::json!({ "triggered": true }))
}

fn spawn_refresh(registry: Shared, region: ServerRegion, trigger: &'static str) {
    tokio::spawn(async move {
        match registry.refresh(region).await {
            Ok(true) => {}
            Ok(false) => info!(
                "{} {}: already up to date",
                region.as_str().to_uppercase(),
                trigger
            ),
            Err(e) => error!(
                "{} {} sync failed (poll will retry): {}",
                region.as_str().to_uppercase(),
                trigger,
                e
            ),
        }
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::Response as HttpResponse;
    use axum::routing::any;

    use super::*;
    use crate::config::{Config, MasterSyncPeer, ServerConfig};
    use crate::updater::sync::{build_syncers, BUNDLE_VERSION_ENTRY};

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("haruki_reg_http_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (format!("http://{address}"), task)
    }

    fn base_config(root: &std::path::Path, token: &str) -> Config {
        let mut config: Config = serde_yaml::from_str("backend: {}").unwrap();
        config.registry.token = token.to_string();
        config.registry.state_dir = root.join("state").to_string_lossy().into_owned();
        let mut server: ServerConfig = serde_yaml::from_str("{}").unwrap();
        server.master_dir = root.join("master").to_string_lossy().into_owned();
        server.version_path = root.join("version.json").to_string_lossy().into_owned();
        std::fs::create_dir_all(&server.master_dir).unwrap();
        config.servers.insert(ServerRegion::Jp, server);
        config
    }

    fn write_version(root: &std::path::Path, data_version: &str) {
        std::fs::write(
            root.join("version.json"),
            format!(
                r#"{{"appVersion":"5.6.0","appHash":"owner-hash","dataVersion":"{data_version}","assetVersion":"5.6.1.10","cdnVersion":0}}"#
            ),
        )
        .unwrap();
    }

    async fn get_json(client: &reqwest::Client, url: &str) -> (u16, serde_json::Value) {
        let resp = client.get(url).send().await.unwrap();
        let status = resp.status().as_u16();
        let body = resp.bytes().await.unwrap();
        (
            status,
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn serves_manifest_files_bundle_and_app_identity_with_token_gating() {
        let root = temp_dir();
        let config = Arc::new(base_config(&root, "secret"));
        std::fs::write(root.join("master/cards.json"), "[{\"id\":1}]").unwrap();
        write_version(&root, "5.6.1.11");
        let registry = Arc::new(Registry::new(config, HashMap::new()));
        registry.publish_missing().await;
        let (base, server) = serve(router(registry.clone())).await;
        let client = reqwest::Client::new();

        let (status, health) = get_json(&client, &format!("{base}/health")).await;
        assert_eq!(status, 200);
        assert_eq!(health["regions"]["jp"]["dataVersion"], "5.6.1.11");
        assert_eq!(health["regions"]["jp"]["synced"], false);

        let (status, current) = get_json(&client, &format!("{base}/v1/master/jp/current")).await;
        assert_eq!(status, 200);
        assert_eq!(current["files"][0]["name"], "cards.json");
        assert_eq!(current["appHash"], "owner-hash");
        let (status, _) = get_json(&client, &format!("{base}/v1/master/kr/current")).await;
        assert_eq!(status, 404);
        let (status, _) = get_json(&client, &format!("{base}/v1/master/xx/current")).await;
        assert_eq!(status, 400);

        let (status, history) =
            get_json(&client, &format!("{base}/v1/master/jp/history?limit=5")).await;
        assert_eq!(status, 200);
        assert_eq!(history.as_array().unwrap().len(), 1);

        let resp = client
            .get(format!("{base}/v1/master/jp/files/cards.json"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "[{\"id\":1}]");
        for bad in [
            "missing.json",
            "..%2Fversion.json",
            "note.txt",
            ".hidden.json",
        ] {
            let resp = client
                .get(format!("{base}/v1/master/jp/files/{bad}"))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 404, "{bad}");
        }

        let resp = client
            .get(format!("{base}/v1/master/jp/bundle"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "application/x-tar");
        assert!(resp.bytes().await.unwrap().len() > 512);

        // App identity falls back to the synced version file...
        let (status, app) = get_json(&client, &format!("{base}/v1/app/jp")).await;
        assert_eq!(status, 200);
        assert_eq!(app["appVersion"], "5.6.0");
        assert_eq!(app["appHash"], "owner-hash");
        // ...until an operator override is stored (token required).
        let body = serde_json::json!({"appVersion": "5.7.0", "appHash": "manual"});
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .body("not json")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let (_, app) = get_json(&client, &format!("{base}/v1/app/jp")).await;
        assert_eq!(app["appVersion"], "5.7.0");
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"appVersion": "", "appHash": ""}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let resp = client
            .delete(format!("{base}/v1/app/jp"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let (_, app) = get_json(&client, &format!("{base}/v1/app/jp")).await;
        assert_eq!(app["appVersion"], "5.6.0");

        // Publish re-scans: a changed file becomes a new history entry.
        std::fs::write(root.join("master/cards.json"), "[{\"id\":2}]").unwrap();
        let resp = client
            .post(format!("{base}/v1/master/jp/publish"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["changed"], true);
        let (_, history) = get_json(&client, &format!("{base}/v1/master/jp/history")).await;
        assert_eq!(history.as_array().unwrap().len(), 2);

        // No syncer for jp: refresh and webhook report not found.
        let resp = client
            .post(format!("{base}/v1/master/jp/refresh"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        let resp = client
            .post(format!("{base}/internal/master-updated"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"server": "jp", "dataVersion": "x"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        server.abort();

        // With no token configured, mutations do not exist.
        let config = Arc::new(base_config(&root, ""));
        let registry = Arc::new(Registry::new(config, HashMap::new()));
        let (base, server) = serve(router(registry)).await;
        let resp = client
            .put(format!("{base}/v1/app/jp"))
            .body("not json")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 404, "disabled endpoints hide body errors");
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Fake owner node: serves the version file and a one-file bundle.
    fn owner_bundle(root: &std::path::Path, data_version: &str) -> Vec<u8> {
        let source = root.join("owner");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("cards.json"), "[{\"id\":7}]").unwrap();
        let version = format!(
            r#"{{"appVersion":"5.6.0","appHash":"owner-hash","dataVersion":"{data_version}","assetVersion":"5.6.1.10","cdnVersion":0}}"#
        );
        std::fs::write(source.join(BUNDLE_VERSION_ENTRY), &version).unwrap();
        let tar_path = root.join("owner.tar");
        let mut builder = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
        builder
            .append_path_with_name(source.join("cards.json"), "cards.json")
            .unwrap();
        builder
            .append_path_with_name(source.join(BUNDLE_VERSION_ENTRY), BUNDLE_VERSION_ENTRY)
            .unwrap();
        builder.finish().unwrap();
        std::fs::read(tar_path).unwrap()
    }

    #[derive(Clone)]
    struct Owner {
        version: Vec<u8>,
        bundle: Vec<u8>,
        hits: Arc<parking_lot::Mutex<Vec<String>>>,
    }

    async fn owner_handler(
        State(owner): State<Owner>,
        uri: axum::http::Uri,
        body: String,
    ) -> HttpResponse<Body> {
        owner.hits.lock().push(format!("{} {}", uri.path(), body));
        let (content_type, bytes) = if uri.path().ends_with("/version") {
            ("application/json", owner.version.clone())
        } else if uri.path().ends_with("/bundle") {
            ("application/x-tar", owner.bundle.clone())
        } else {
            ("application/json", b"{\"ok\":true}".to_vec())
        };
        HttpResponse::builder()
            .header("content-type", content_type)
            .body(Body::from(bytes))
            .unwrap()
    }

    #[tokio::test]
    async fn webhook_pulls_from_owner_publishes_and_notifies_subscribers() {
        let root = temp_dir();
        let version = r#"{"appVersion":"5.6.0","appHash":"owner-hash","dataVersion":"9.0.0.1","assetVersion":"9.0.0.1","cdnVersion":0}"#.to_string();
        let owner = Owner {
            version: version.into_bytes(),
            bundle: owner_bundle(&root, "9.0.0.1"),
            hits: Arc::new(parking_lot::Mutex::new(Vec::new())),
        };
        let owner_app = Router::new()
            .fallback(any(owner_handler))
            .with_state(owner.clone());
        let (owner_url, owner_server) = serve(owner_app).await;

        let mut config = base_config(&root, "secret");
        {
            let server = config.servers.get_mut(&ServerRegion::Jp).unwrap();
            server.master_sync.source_url = owner_url.clone();
            server.master_sync.source_token = "owner-token".to_string();
        }
        // The owner doubles as the subscriber so the notice lands in `hits`.
        config.registry.subscribers = vec![MasterSyncPeer {
            url: owner_url.clone(),
            token: "sub-token".to_string(),
        }];
        let version_locks = HashMap::new();
        let syncers = build_syncers(&config, &HashMap::new(), None, &version_locks);
        assert!(syncers.contains_key(&ServerRegion::Jp));
        let registry = Arc::new(Registry::new(Arc::new(config), syncers));
        let (base, server) = serve(router(registry.clone())).await;
        let client = reqwest::Client::new();

        let (status, _) = get_json(&client, &format!("{base}/v1/master/jp/current")).await;
        assert_eq!(status, 404);
        let resp = client
            .post(format!("{base}/internal/master-updated"))
            .bearer_auth("secret")
            .json(&serde_json::json!({"server": "jp", "dataVersion": "9.0.0.1"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let mut published = None;
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Ok(Some(m)) = registry.state.current(ServerRegion::Jp).await {
                published = Some(m);
                break;
            }
        }
        let manifest = published.expect("webhook sync should publish");
        assert_eq!(manifest.data_version, "9.0.0.1");
        assert_eq!(manifest.files[0].name, "cards.json");
        assert!(root.join("master/cards.json").exists());
        let hits = owner.hits.lock().clone();
        assert!(hits
            .iter()
            .any(|h| h.contains("/internal/master/jp/version")));
        assert!(hits
            .iter()
            .any(|h| h.contains("/internal/master/jp/bundle")));
        assert!(hits
            .iter()
            .any(|h| h.contains("/internal/master-updated") && h.contains("9.0.0.1")));

        // A second refresh finds nothing new and publishes nothing.
        assert!(!registry.refresh(ServerRegion::Jp).await.unwrap());
        let (_, history) = get_json(&client, &format!("{base}/v1/master/jp/history")).await;
        assert_eq!(history.as_array().unwrap().len(), 1);

        let resp = client
            .post(format!("{base}/v1/master/jp/refresh"))
            .bearer_auth("secret")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        server.abort();
        owner_server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }
}

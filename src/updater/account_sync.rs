//! Central account distribution (master side).
//!
//! The node that owns a region's account files pushes the whole set to the
//! standby nodes listed in `servers.<region>.account_sync.peers`: at startup,
//! after every account reload, and on a slow timer so a standby that was down
//! catches up. Standbys store the set and stay logged out (see
//! `SekaiClient::import_accounts`); if the master dies they already hold what
//! they need. Credentials only ever travel master -> standby.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::client::SekaiClient;
use crate::config::{AccountSyncPeer, Config, ServerRegion};
use crate::error::AppError;
use crate::upstream::{AccountPushRequest, InternalApiResponse};

/// Re-push interval when every standby already has the current set.
const RESYNC_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// Retry interval after a failed push.
const RETRY_INTERVAL: Duration = Duration::from_secs(60);

fn peer_name(peer: &AccountSyncPeer) -> &str {
    if peer.name.is_empty() {
        &peer.url
    } else {
        &peer.name
    }
}

/// Start one push task per region that lists standby peers.
pub fn start(
    config: &Config,
    clients: &HashMap<ServerRegion, Arc<SekaiClient>>,
) -> Result<(), AppError> {
    let mut http = None;
    for (region, server) in &config.servers {
        let sync = &server.account_sync;
        if sync.peers.is_empty() {
            continue;
        }
        let tag = region.as_str().to_uppercase();
        if sync.receive {
            warn!(
                "{} account_sync.peers ignored: this node receives the region's accounts",
                tag
            );
            continue;
        }
        let Some(client) = clients.get(region) else {
            warn!(
                "{} account_sync.peers ignored: no local accounts for the region",
                tag
            );
            continue;
        };
        let http = match &http {
            Some(http) => http,
            None => http.insert(build_push_client()?),
        }
        .clone();
        let (client, peers) = (client.clone(), sync.peers.clone());
        tokio::spawn(async move {
            loop {
                let delivered = push_to_peers(&client, &peers, &http).await;
                let wait = if delivered {
                    RESYNC_INTERVAL
                } else {
                    RETRY_INTERVAL
                };
                tokio::select! {
                    _ = client.accounts_changed() => {}
                    _ = tokio::time::sleep(wait) => {}
                }
            }
        });
    }
    Ok(())
}

/// HTTP client for account pushes. Redirects are refused: a 307/308 would
/// replay the request body, which is the account set, to wherever the peer
/// (or something answering in its place) points.
pub fn build_push_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::NetworkError(e.to_string()))
}

/// Push the region's current account set to every peer. Returns whether all
/// of them now hold it.
pub async fn push_to_peers(
    client: &SekaiClient,
    peers: &[AccountSyncPeer],
    http: &reqwest::Client,
) -> bool {
    let tag = client.region.as_str().to_uppercase();
    let (accounts, clear) = match client.export_accounts() {
        Ok((accounts, _)) if !accounts.is_empty() => (accounts, false),
        // The directory is there and holds no account file at all: the
        // accounts were removed on purpose, and the standbys must follow.
        Ok((accounts, Some(0))) => (accounts, true),
        // A missing directory, or files that yield no usable account, look
        // like a broken mount or a bad edit. Never wipe the standbys on that.
        Ok(_) => {
            warn!("{} No usable accounts to push to standby nodes", tag);
            return false;
        }
        Err(e) => {
            warn!("{} Could not read accounts for standby push: {}", tag, e);
            return false;
        }
    };
    let request = AccountPushRequest {
        server: client.region.as_str().to_string(),
        accounts,
        clear,
    };
    let mut delivered = true;
    for peer in peers {
        match push_one(http, peer, &request).await {
            Ok(true) => info!(
                "{} Pushed {} account(s) to standby '{}'",
                tag,
                request.accounts.len(),
                peer_name(peer)
            ),
            Ok(false) => debug!("{} Standby '{}' is up to date", tag, peer_name(peer)),
            Err(e) => {
                delivered = false;
                warn!(
                    "{} Account push to standby '{}' failed: {}",
                    tag,
                    peer_name(peer),
                    e
                );
            }
        }
    }
    delivered
}

/// Returns whether the standby's stored set changed.
async fn push_one(
    http: &reqwest::Client,
    peer: &AccountSyncPeer,
    request: &AccountPushRequest,
) -> Result<bool, AppError> {
    let response = http
        .post(format!(
            "{}/internal/accounts",
            peer.url.trim_end_matches('/')
        ))
        .bearer_auth(&peer.token)
        .json(request)
        .send()
        .await
        .map_err(|e| AppError::NetworkError(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::NetworkError(format!("standby returned {status}")));
    }
    let envelope: InternalApiResponse = response
        .json()
        .await
        .map_err(|e| AppError::NetworkError(format!("invalid envelope: {e}")))?;
    if !envelope.ok {
        return Err(AppError::from_kind(
            envelope.kind.as_deref().unwrap_or(""),
            envelope.status,
            envelope.message.unwrap_or_default(),
        ));
    }
    Ok(envelope
        .data
        .and_then(|data| data.get("changed").and_then(|changed| changed.as_bool()))
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::{AppState, RequestCoalescer};

    const KEY: &str = "00112233445566778899aabbccddeeff";
    const IV: &str = "ffeeddccbbaa99887766554433221100";

    async fn client(root: &std::path::Path, receive: bool) -> Arc<SekaiClient> {
        let mut config: ServerConfig = serde_yaml::from_str("{}").unwrap();
        config.account_dir = root.join("accounts").to_string_lossy().into_owned();
        config.version_path = root.join("version.json").to_string_lossy().into_owned();
        config.aes_key_hex = KEY.to_string();
        config.aes_iv_hex = IV.to_string();
        config.account_sync.receive = receive;
        std::fs::create_dir_all(&config.account_dir).unwrap();
        Arc::new(
            SekaiClient::new(
                ServerRegion::En,
                config,
                None,
                None,
                SekaiClient::build_http_client(None).unwrap(),
                None,
            )
            .await
            .unwrap(),
        )
    }

    async fn serve_standby(standby: Arc<SekaiClient>) -> (String, tokio::task::JoinHandle<()>) {
        let state = Arc::new(AppState {
            config: serde_yaml::from_str("backend:\n  internal_token: secret").unwrap(),
            clients: HashMap::from([(ServerRegion::En, standby)]),
            routers: HashMap::new(),
            syncers: HashMap::new(),
            version_locks: HashMap::new(),
            db: None,
            master_db: None,
            redis: None,
            jwt_secret: None,
            coalescer: Arc::new(RequestCoalescer::default()),
        });
        let app = crate::api::create_router(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, task)
    }

    fn peer(url: &str, token: &str) -> AccountSyncPeer {
        AccountSyncPeer {
            url: url.to_string(),
            token: token.to_string(),
            name: String::new(),
        }
    }

    // The whole path: the master reads its account files, the standby's
    // internal endpoint stores them, and the standby loads them logged out.
    #[tokio::test]
    async fn master_pushes_accounts_to_a_standby_over_the_internal_api() {
        let root =
            std::env::temp_dir().join(format!("haruki_account_sync_{}", uuid::Uuid::new_v4()));
        let master = client(&root.join("master"), false).await;
        let standby = client(&root.join("standby"), true).await;
        let (url, server) = serve_standby(standby.clone()).await;
        let http = build_push_client().unwrap();

        // An empty account directory is a confirmed empty set: pushed as a
        // clear, which is a no-op on a standby that holds nothing.
        assert!(push_to_peers(&master, &[peer(&url, "secret")], &http).await);
        assert_eq!(standby.session_count(), 0);
        // Files that yield no usable account are not: nothing is pushed.
        std::fs::write(root.join("master/accounts/broken.json"), "{").unwrap();
        assert!(!push_to_peers(&master, &[peer(&url, "secret")], &http).await);
        std::fs::remove_file(root.join("master/accounts/broken.json")).unwrap();
        for user in ["10", "11"] {
            std::fs::write(
                root.join(format!("master/accounts/{user}.json")),
                format!(r#"{{"userId":"{user}","deviceId":"d","credential":"c"}}"#),
            )
            .unwrap();
        }
        assert!(push_to_peers(&master, &[peer(&url, "secret")], &http).await);
        assert!(
            push_to_peers(&master, &[peer(&url, "secret")], &http).await,
            "a repeated push is a no-op that still counts as delivered"
        );
        // Loaded before the push was acknowledged, without a watcher.
        assert_eq!(standby.session_count(), 2);
        assert!(standby.get_session().unwrap().needs_login());

        // A redirecting peer must not get the account set replayed elsewhere.
        let redirector = axum::Router::new().fallback(|| async {
            (
                axum::http::StatusCode::TEMPORARY_REDIRECT,
                [("location", "http://127.0.0.1:1/internal/accounts")],
            )
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_url = format!("http://{}", listener.local_addr().unwrap());
        let redirect_server =
            tokio::spawn(async move { axum::serve(listener, redirector).await.unwrap() });
        assert!(!push_to_peers(&master, &[peer(&redirect_url, "secret")], &http).await);
        redirect_server.abort();

        // The operator removes every account on the master: the standby's
        // copy goes too.
        for user in ["10", "11"] {
            std::fs::remove_file(root.join(format!("master/accounts/{user}.json"))).unwrap();
        }
        assert!(push_to_peers(&master, &[peer(&url, "secret")], &http).await);
        assert_eq!(standby.session_count(), 0);
        assert!(!root
            .join("standby/accounts")
            .join(crate::client::sekai_client::SYNCED_ACCOUNTS_FILE)
            .exists());
        for user in ["10", "11"] {
            std::fs::write(
                root.join(format!("master/accounts/{user}.json")),
                format!(r#"{{"userId":"{user}","deviceId":"d","credential":"c"}}"#),
            )
            .unwrap();
        }
        assert!(push_to_peers(&master, &[peer(&url, "secret")], &http).await);
        assert_eq!(standby.session_count(), 2);

        // Wrong token, unreachable peer: reported, never fatal.
        assert!(!push_to_peers(&master, &[peer(&url, "wrong")], &http).await);
        assert!(!push_to_peers(&master, &[peer("http://127.0.0.1:1", "secret")], &http).await);
        // A node that does not opt in refuses the set.
        let (plain_url, plain_server) = serve_standby(master.clone()).await;
        assert!(!push_to_peers(&master, &[peer(&plain_url, "secret")], &http).await);

        server.abort();
        plain_server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn start_skips_regions_that_cannot_push() {
        let root =
            std::env::temp_dir().join(format!("haruki_account_sync_{}", uuid::Uuid::new_v4()));
        let mut config: Config = serde_yaml::from_str(
            r#"
backend: {}
servers:
  en:
    account_sync:
      peers: [{url: "http://127.0.0.1:1"}]
      receive: true
  jp:
    account_sync:
      peers: [{url: "http://127.0.0.1:1", name: standby}]
  kr: {}
"#,
        )
        .unwrap();
        assert_eq!(
            peer_name(&config.servers[&ServerRegion::Jp].account_sync.peers[0]),
            "standby"
        );
        assert_eq!(
            peer_name(&config.servers[&ServerRegion::En].account_sync.peers[0]),
            "http://127.0.0.1:1"
        );
        // en receives, jp has no local client, kr lists no peers: nothing starts.
        let clients = HashMap::from([(ServerRegion::En, client(&root, true).await)]);
        start(&config, &clients).unwrap();
        // With a pushing region the task starts and survives an unreachable peer.
        config
            .servers
            .get_mut(&ServerRegion::En)
            .unwrap()
            .account_sync
            .receive = false;
        start(&config, &clients).unwrap();
        tokio::task::yield_now().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}

//! Shared startup helpers used by every binary in this crate.
//!
//! Both `haruki-sekai-api` (HTTP server) and `haruki-sekai-updater`
//! (cron-only worker) load the same [`Config`], spin up the same set of
//! [`SekaiClient`]s, and connect to the same databases. Centralizing that
//! sequence here keeps the binaries thin and prevents init drift.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::signal;
use tracing::{error, info, warn};

use crate::client::SekaiClient;
use crate::config::Config;
use crate::db;
use crate::error::AppError;
use crate::AppState;

pub async fn init_app_state(config: Config) -> anyhow::Result<AppState> {
    let mut clients = HashMap::new();
    let jp_cookie_url = if config.jp_sekai_cookie_url.is_empty() {
        None
    } else {
        Some(config.jp_sekai_cookie_url.clone())
    };
    let mut init_tasks = Vec::new();
    for (region, server_config) in &config.servers {
        if server_config.enabled {
            let region = *region;
            let server_config = server_config.clone();
            let proxy = if config.proxy.is_empty() {
                None
            } else {
                Some(config.proxy.clone())
            };
            let jp_cookie_url = jp_cookie_url.clone();

            init_tasks.push(tokio::spawn(async move {
                info!("Initializing {} server...", region.as_str().to_uppercase());
                let client = SekaiClient::new(region, server_config, proxy, jp_cookie_url).await?;
                client.init().await?;
                Ok::<_, AppError>((region, Arc::new(client)))
            }));
        }
    }
    let results = futures::future::join_all(init_tasks).await;
    for result in results {
        match result {
            Ok(Ok((region, client))) => {
                if let Err(e) = client.clone().start_file_watcher() {
                    warn!(
                        "Failed to start file watcher for {}: {}",
                        region.as_str(),
                        e
                    );
                }
                clients.insert(region, client);
            }
            Ok(Err(e)) => {
                error!("Failed to initialize server: {}", e);
            }
            Err(e) => {
                error!("Task panicked: {}", e);
            }
        }
    }
    let db = if config.database.enabled {
        Some(db::init_db(&config.database).await?)
    } else {
        None
    };
    let redis = if config.redis.enabled {
        Some(db::init_redis(&config.redis).await?)
    } else {
        None
    };
    let master_db = if config.master_database.enabled {
        Some(db::init_master_db(&config.master_database).await?)
    } else {
        None
    };
    let jwt_secret = if config.backend.sekai_user_jwt_signing_key.is_empty() {
        None
    } else {
        Some(config.backend.sekai_user_jwt_signing_key.clone())
    };
    Ok(AppState {
        config,
        clients,
        db,
        master_db,
        redis,
        jwt_secret,
    })
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Shutdown signal received, starting graceful shutdown...");
}

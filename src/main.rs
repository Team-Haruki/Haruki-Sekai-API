mod api;
mod client;
mod config;
mod crypto;
mod db;
mod error;
mod updater;
mod utils;

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::signal;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::api::create_router;
use crate::client::SekaiClient;
use crate::config::Config;
use crate::error::AppError;

pub struct AppState {
    pub config: Config,
    pub clients: std::collections::HashMap<config::ServerRegion, Arc<SekaiClient>>,
    pub db: Option<sea_orm::DatabaseConnection>,
    pub redis: Option<redis::aio::ConnectionManager>,
    pub jwt_secret: Option<String>,
}

struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(
            w,
            "{}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%z")
        )
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_timer(LocalTimer)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    info!(
        "========================= Haruki Sekai API v{} =========================",
        env!("CARGO_PKG_VERSION")
    );
    info!("Powered By Haruki Dev Team");
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };
    let state = match init_app_state(config).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            error!("Failed to initialize application: {}", e);
            std::process::exit(1);
        }
    };
    let app = create_router(state.clone());
    let _scheduler = match updater::start_scheduler(&state.clients, &state.config).await {
        Ok(s) => Some(s),
        Err(e) => {
            error!("Failed to start scheduler: {}", e);
            None
        }
    };
    let addr: SocketAddr = format!(
        "{}:{}",
        state.config.backend.host, state.config.backend.port
    )
    .parse()
    .expect("Invalid address");
    info!("Starting HTTP server at {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("Server shutdown complete");
    Ok(())
}

async fn init_app_state(config: Config) -> anyhow::Result<AppState> {
    use std::collections::HashMap;
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
    let jwt_secret = if config.backend.sekai_user_jwt_signing_key.is_empty() {
        None
    } else {
        Some(config.backend.sekai_user_jwt_signing_key.clone())
    };
    Ok(AppState {
        config,
        clients,
        db,
        redis,
        jwt_secret,
    })
}

async fn shutdown_signal() {
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

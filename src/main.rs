use std::net::SocketAddr;
use std::sync::Arc;

use tokio::signal;
use tracing::{error, info, warn};

use haruki_sekai_api::api::create_router;
use haruki_sekai_api::client::nuverse_schema::NuverseSchemaStore;
use haruki_sekai_api::client::SekaiClient;
use haruki_sekai_api::config::Config;
use haruki_sekai_api::db;
use haruki_sekai_api::error::AppError;
use haruki_sekai_api::updater;

use haruki_sekai_api::AppState;

mod logging;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init();

    info!(
        "========================= Haruki Sekai API v{} =========================",
        env!("CARGO_PKG_VERSION")
    );
    info!("Powered by Haruki Dev Team");
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
    let _scheduler = match updater::start_scheduler(
        &state.clients,
        &state.config,
        state.master_db.clone(),
    )
    .await
    {
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
    // Resources shared across all regions, built once and handed into each client:
    // one reqwest client (only the global proxy varies) and one NuverseSchemaStore
    // per distinct bundle path (Tw/Kr/Cn share the same file, so parse it once).
    let proxy_opt = if config.proxy.is_empty() {
        None
    } else {
        Some(config.proxy.clone())
    };
    let http_client = SekaiClient::build_http_client(proxy_opt.as_deref())?;
    let mut schema_cache: HashMap<String, Arc<NuverseSchemaStore>> = HashMap::new();
    for (region, server_config) in &config.servers {
        let path = &server_config.nuverse_schema_bundle_path;
        if server_config.enabled
            && !region.is_cp_server()
            && !path.is_empty()
            && !schema_cache.contains_key(path)
        {
            match SekaiClient::load_nuverse_schema_store(path) {
                Ok(store) => {
                    schema_cache.insert(path.clone(), Arc::new(store));
                }
                Err(e) => error!("Failed to load Nuverse schema bundle {}: {}", path, e),
            }
        }
    }

    let mut init_tasks = Vec::new();
    for (region, server_config) in &config.servers {
        if server_config.enabled {
            let region = *region;
            let server_config = server_config.clone();
            let proxy = proxy_opt.clone();
            let jp_cookie_url = jp_cookie_url.clone();
            let http_client = http_client.clone();
            let needs_store =
                !region.is_cp_server() && !server_config.nuverse_schema_bundle_path.is_empty();
            let nuverse_store = if needs_store {
                schema_cache
                    .get(&server_config.nuverse_schema_bundle_path)
                    .cloned()
            } else {
                None
            };

            init_tasks.push(tokio::spawn(async move {
                info!("Initializing {} server...", region.as_str().to_uppercase());
                if needs_store && nuverse_store.is_none() {
                    return Err(AppError::IoError(format!(
                        "Nuverse schema bundle not loaded for {}",
                        region.as_str()
                    )));
                }
                let client = SekaiClient::new(
                    region,
                    server_config,
                    proxy,
                    jp_cookie_url,
                    http_client,
                    nuverse_store,
                )
                .await?;
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
        coalescer: Arc::new(haruki_sekai_api::RequestCoalescer::default()),
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

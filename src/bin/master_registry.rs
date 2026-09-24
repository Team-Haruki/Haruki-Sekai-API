//! Master data manager. Reuses the SekaiAPI config file: every region with a
//! `master_dir` is managed; regions with `master_sync.source_url` are pulled
//! from that owner node; `git` and `master_database` drive push and ingest;
//! the `registry` section configures the HTTP surface and subscribers.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tracing::{error, info};

use haruki_sekai_api::config::Config;
use haruki_sekai_api::db;
use haruki_sekai_api::registry::blobs::open_blob_store;
use haruki_sekai_api::registry::state::RegistryState;
use haruki_sekai_api::registry::{http, Registry};
use haruki_sekai_api::updater::sync::build_syncers;

#[path = "../logging.rs"]
mod logging;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    logging::init(&config.backend.log_level);
    info!(
        "Haruki Master Registry v{} starting",
        env!("CARGO_PKG_VERSION")
    );

    let master_db = if config.master_database.enabled {
        Some(db::init_master_db(&config.master_database).await?)
    } else {
        None
    };
    let version_locks: HashMap<_, _> = config
        .servers
        .keys()
        .map(|region| (*region, Arc::new(tokio::sync::Mutex::new(()))))
        .collect();
    let syncers = build_syncers(&config, &HashMap::new(), master_db, &version_locks);
    let mut state = if config.registry.state_dsn.trim().is_empty() {
        info!("Registry state: files under {}", config.registry.state_dir);
        RegistryState::new(&config.registry.state_dir)
    } else {
        let state =
            RegistryState::connect(&config.registry.state_dir, &config.registry.state_dsn).await?;
        info!("Registry state: database (registry.state_dsn)");
        state
    };
    let blobs = open_blob_store(&config.registry, &mut state).await?;
    info!("Registry blob store: {:?}", config.registry.blob_store);
    let config = Arc::new(config);
    let registry =
        Arc::new(Registry::with_state(config.clone(), syncers, state).with_blob_store(blobs));
    for region in registry.regions() {
        info!(
            "{} managed (owner: {})",
            region.as_str().to_uppercase(),
            if registry.syncers.contains_key(&region) {
                config.servers[&region].master_sync.source_url.as_str()
            } else {
                "<none, local files only>"
            }
        );
    }
    registry.publish_missing().await;
    // Blobs of existing manifests (first start on `blob_store: pg`): in the
    // background, reads fall back to the master directories meanwhile.
    tokio::spawn({
        let registry = registry.clone();
        async move {
            registry.import_blobs().await;
        }
    });
    let _scheduler = registry.start_polls().await?;

    let addr: SocketAddr = format!("{}:{}", config.registry.host, config.registry.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Registry listening on {}", addr);
    let app = http::router(registry);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if let Err(e) = tokio::signal::ctrl_c().await {
                error!("Failed to listen for shutdown signal: {}", e);
            }
            info!("Shutdown signal received");
        })
        .await?;
    Ok(())
}

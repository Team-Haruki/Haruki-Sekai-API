//! Registry-driven master ingest. Reuses the SekaiAPI config file (or a
//! minimal one with `backend: {}`): the `ingest` section selects the registry
//! and the database targets. Reads only the registry's HTTP surface; see
//! `docs/master-registry-storage-and-ingest.md`, part B.

use std::net::SocketAddr;
use std::sync::Arc;

use tracing::{error, info, warn};

use haruki_sekai_api::config::Config;
use haruki_sekai_api::ingest::{self, Ingester};

#[path = "../logging.rs"]
mod logging;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    logging::init(&config.backend.log_level);
    info!(
        "Haruki Master Ingest v{} starting",
        env!("CARGO_PKG_VERSION")
    );
    let Some(ingest_config) = config.ingest.clone() else {
        anyhow::bail!("the config has no `ingest` section");
    };
    if config.master_database.enabled
        && ingest_config
            .targets
            .iter()
            .any(|t| t.dsn == config.master_database.dsn)
    {
        warn!(
            "master_database (the in-sync ingest) points at an ingest target: disable one of the two \
             so a database is not written by both"
        );
    }
    let ingester = match Ingester::new(ingest_config).await {
        Ok(i) => Arc::new(i),
        Err(e) => ingest::fatal(&e),
    };
    for target in &ingester.targets {
        info!(
            "Ingest target {}: regions {:?}, raw {}, create_tables {}",
            target.name,
            target
                .cfg
                .regions
                .iter()
                .map(|r| r.as_str())
                .collect::<Vec<_>>(),
            target.cfg.raw,
            target.cfg.create_tables
        );
    }
    if let Err(e) = ingester.check_registry().await {
        ingest::fatal(&e);
    }
    ingester.trigger_all();
    let _scheduler = ingester.start_timer().await?;

    let addr: SocketAddr = ingester.config.listen.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Ingest listening on {}", addr);
    axum::serve(listener, ingest::http::router(ingester))
        .with_graceful_shutdown(async {
            if let Err(e) = tokio::signal::ctrl_c().await {
                error!("Failed to listen for shutdown signal: {}", e);
            }
            info!("Shutdown signal received");
        })
        .await?;
    Ok(())
}

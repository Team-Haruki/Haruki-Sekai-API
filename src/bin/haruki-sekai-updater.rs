//! Standalone binary that runs only the master/apphash/cookie updaters.
//!
//! In Kubernetes, run this as a separate `replicas: 1` Deployment so the API
//! Deployment can scale horizontally without each replica racing on the same
//! cron jobs (master data download + git push). The `backend.run_updaters_inproc`
//! flag in the API config should be set to `false` in that topology.

use std::sync::Arc;

use tracing::{error, info};

use haruki_sekai_api::bootstrap::{init_app_state, shutdown_signal};
use haruki_sekai_api::config::Config;
use haruki_sekai_api::logging;
use haruki_sekai_api::updater;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };
    logging::init(&config.backend.log_format);

    info!(
        "========================= Haruki Sekai Updater v{} =========================",
        env!("CARGO_PKG_VERSION")
    );

    let state = match init_app_state(config).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            error!("Failed to initialize application: {}", e);
            std::process::exit(1);
        }
    };

    // Always run the scheduler in this binary, regardless of run_updaters_inproc.
    let _scheduler = match updater::start_scheduler(
        &state.clients,
        &state.config,
        state.master_db.clone(),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to start scheduler: {}", e);
            std::process::exit(1);
        }
    };

    info!("Updater running; waiting for shutdown signal");
    shutdown_signal().await;
    info!("Updater shutdown complete");
    Ok(())
}

use std::net::SocketAddr;
use std::sync::Arc;

use tracing::{error, info};

use haruki_sekai_api::api::create_router;
use haruki_sekai_api::bootstrap::{init_app_state, shutdown_signal};
use haruki_sekai_api::config::Config;
use haruki_sekai_api::logging;
use haruki_sekai_api::updater;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Config first, logging second — see logging::init for the rationale.
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };
    logging::init(&config.backend.log_format);

    info!(
        "========================= Haruki Sekai API v{} =========================",
        env!("CARGO_PKG_VERSION")
    );
    info!("Powered By Haruki Dev Team");

    let run_updaters_inproc = config.backend.run_updaters_inproc;
    let state = match init_app_state(config).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            error!("Failed to initialize application: {}", e);
            std::process::exit(1);
        }
    };
    let app = create_router(state.clone());

    let _scheduler = if run_updaters_inproc {
        match updater::start_scheduler(
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
        }
    } else {
        info!(
            "In-process updaters disabled (backend.run_updaters_inproc=false); \
             expecting an external haruki-sekai-updater instance"
        );
        None
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

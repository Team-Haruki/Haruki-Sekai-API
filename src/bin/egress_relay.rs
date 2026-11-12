//! Egress relay role. Reuses the SekaiAPI config file (or a minimal one with
//! `backend: {}`): the `egress_relay` section holds the listen address, the
//! token and the allowed game hosts. See `haruki_sekai_api::relay`.

use std::net::SocketAddr;
use std::sync::Arc;

use tracing::{error, info};

use haruki_sekai_api::config::Config;
use haruki_sekai_api::relay::{self, Relay};

#[path = "../logging.rs"]
mod logging;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    logging::init(&config.backend.log_level);
    info!(
        "Haruki Egress Relay v{} starting",
        env!("CARGO_PKG_VERSION")
    );
    let Some(relay_config) = config.egress_relay.clone() else {
        anyhow::bail!("the config has no `egress_relay` section");
    };
    let relay = Arc::new(Relay::new(&relay_config).map_err(anyhow::Error::msg)?);
    let addr: SocketAddr = relay_config.listen.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(
        "Egress relay listening on {} for {} host(s)",
        addr,
        relay_config.allow_hosts.len()
    );
    relay::serve(listener, relay, async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            error!("Failed to listen for shutdown signal: {}", e);
        }
        info!("Shutdown signal received");
    })
    .await;
    Ok(())
}

use haruki_sekai_api::config::Config;
use haruki_sekai_api::ingest_engine::IngestionEngine;
use sea_orm::{ConnectOptions, Database};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    let config = Config::load()?;
    let master_db_config = &config.master_database;
    if !master_db_config.enabled {
        anyhow::bail!("master_database.enabled is false in the selected config");
    }
    if master_db_config.dsn.is_empty() {
        anyhow::bail!("master_database.dsn is empty in the selected config");
    }

    println!("Connecting to configured master PostgreSQL database...");
    let mut opt = ConnectOptions::new(master_db_config.dsn.clone());
    opt.max_connections(master_db_config.max_connections.max(1))
        .min_connections(1)
        .connect_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(8))
        .sqlx_logging(true);

    let db = Database::connect(opt).await?;
    println!("Connected! Initializing engine...");

    // Engine initialization
    let engine = IngestionEngine::new(db).await?;

    // Traverse and ingest all regions
    for region in [
        haruki_sekai_api::config::ServerRegion::Jp,
        haruki_sekai_api::config::ServerRegion::En,
        haruki_sekai_api::config::ServerRegion::Tw,
        haruki_sekai_api::config::ServerRegion::Kr,
        haruki_sekai_api::config::ServerRegion::Cn,
    ] {
        let Some(server) = config.servers.get(&region) else {
            continue;
        };
        if !server.enabled || server.master_dir.is_empty() {
            continue;
        }
        // Same layout as the master updater: files live directly in master_dir.
        let path = server.master_dir.trim_end_matches('/').to_string();
        println!("Ingesting {} region data from {}...", region.as_str(), path);
        // Tolerate per-region failures so one region with bad files does not abort
        // ingestion for the others; ingest_master_data now errors on any bad file.
        if let Err(e) = engine.ingest_master_data(&path, region.as_str()).await {
            eprintln!("Ingestion failed for {}: {e:#}", region.as_str());
        }
    }
    Ok(())
}

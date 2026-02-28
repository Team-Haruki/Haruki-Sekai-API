use haruki_sekai_api::ingest_engine::IngestionEngine;
use sea_orm::{ConnectOptions, Database};
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    println!("Connecting to PostgreSQL...");
    let mut opt =
        ConnectOptions::new("postgres://haruki:sekai@localhost:5432/master_data".to_owned());
    opt.max_connections(5)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(8))
        .sqlx_logging(true);

    let db = Database::connect(opt).await?;
    println!("Connected! Initializing engine...");

    // Engine initialization
    let engine = IngestionEngine::new(db).await?;

    // Traverse and ingest all regions
    let regions = vec![
        ("Data/master/haruki-sekai-master/master", "jp"),
        ("Data/master/haruki-sekai-en-master/master", "en"),
        ("Data/master/haruki-sekai-tc-master/master", "tw"),
        ("Data/master/haruki-sekai-kr-master/master", "kr"),
        ("Data/master/haruki-sekai-sc-master/master", "cn"),
    ];

    for (path, region) in regions {
        println!("Ingesting {} region data from {}...", region, path);
        engine.ingest_master_data(path, region).await?;
    }
    Ok(())
}

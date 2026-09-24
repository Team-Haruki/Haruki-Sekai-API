pub mod entity;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Schema};
use tracing::info;

use crate::config::{DatabaseConfig, RedisConfig};
use crate::error::AppError;

pub async fn init_db(config: &DatabaseConfig) -> Result<DatabaseConnection, AppError> {
    if config.dsn.is_empty() {
        return Err(AppError::DatabaseError("Database DSN is empty".to_string()));
    }
    let mut opts = ConnectOptions::new(&config.dsn);
    opts.max_connections(config.max_connections.max(1))
        .min_connections(1)
        .connect_timeout(std::time::Duration::from_secs(30))
        .acquire_timeout(std::time::Duration::from_secs(30))
        .sqlx_logging(false);

    let db = Database::connect(opts)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to connect to database: {}", e)))?;

    create_tables(&db).await?;
    info!("Database initialized successfully (SeaORM)");
    Ok(db)
}

async fn create_tables(db: &DatabaseConnection) -> Result<(), AppError> {
    let backend = db.get_database_backend();
    let schema = Schema::new(backend);
    let stmt = schema
        .create_table_from_entity(entity::SekaiUser)
        .if_not_exists()
        .to_owned();
    db.execute(&stmt)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to create sekai_users: {}", e)))?;
    let stmt = schema
        .create_table_from_entity(entity::SekaiUserServer)
        .if_not_exists()
        .to_owned();
    db.execute(&stmt).await.map_err(|e| {
        AppError::DatabaseError(format!("Failed to create sekai_user_servers: {}", e))
    })?;
    Ok(())
}

pub async fn init_master_db(config: &DatabaseConfig) -> Result<DatabaseConnection, AppError> {
    if config.dsn.is_empty() {
        return Err(AppError::DatabaseError(
            "Master Database DSN is empty".to_string(),
        ));
    }
    let mut opts = ConnectOptions::new(&config.dsn);
    opts.max_connections(config.max_connections.max(1))
        .min_connections(1)
        .connect_timeout(std::time::Duration::from_secs(30))
        .acquire_timeout(std::time::Duration::from_secs(30))
        .sqlx_logging(false);

    let db = Database::connect(opts).await.map_err(|e| {
        AppError::DatabaseError(format!("Failed to connect to master database: {}", e))
    })?;

    info!("Master Database initialized successfully (SeaORM)");
    Ok(db)
}

/// Connect to the registry state database (`registry.state_dsn`) and create
/// its tables when missing. Any SeaORM backend works; PostgreSQL is the
/// production target (the value columns are JSONB there).
pub async fn init_registry_state_db(dsn: &str) -> Result<DatabaseConnection, AppError> {
    if dsn.trim().is_empty() {
        return Err(AppError::DatabaseError(
            "Registry state DSN is empty".to_string(),
        ));
    }
    let mut opts = ConnectOptions::new(dsn.trim());
    // Blob reads (`registry.blob_store: pg`) share the pool; they are bounded
    // separately so state writes always find a connection.
    opts.max_connections(8)
        .min_connections(1)
        .connect_timeout(std::time::Duration::from_secs(30))
        .acquire_timeout(std::time::Duration::from_secs(30))
        .sqlx_logging(false);
    let db = Database::connect(opts).await.map_err(|e| {
        AppError::DatabaseError(format!(
            "Failed to connect to registry state database: {}",
            e
        ))
    })?;
    let schema = Schema::new(db.get_database_backend());
    let stmt = schema
        .create_table_from_entity(entity::RegistryStateEntry)
        .if_not_exists()
        .to_owned();
    db.execute(&stmt)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to create registry_state: {}", e)))?;
    let stmt = schema
        .create_table_from_entity(entity::RegistryPublishHistory)
        .if_not_exists()
        .to_owned();
    db.execute(&stmt).await.map_err(|e| {
        AppError::DatabaseError(format!("Failed to create registry_publish_history: {}", e))
    })?;
    for mut stmt in schema.create_index_from_entity(entity::RegistryPublishHistory) {
        db.execute(stmt.if_not_exists()).await.map_err(|e| {
            AppError::DatabaseError(format!(
                "Failed to create registry_publish_history index: {}",
                e
            ))
        })?;
    }
    info!("Registry state database initialized successfully (SeaORM)");
    Ok(db)
}

/// Create the registry blob table (`registry.blob_store: pg`) on the
/// registry state database when missing.
pub async fn init_registry_blob_table(db: &DatabaseConnection) -> Result<(), AppError> {
    let schema = Schema::new(db.get_database_backend());
    let stmt = schema
        .create_table_from_entity(entity::RegistryBlob)
        .if_not_exists()
        .to_owned();
    db.execute(&stmt)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to create registry_blobs: {}", e)))?;
    for mut stmt in schema.create_index_from_entity(entity::RegistryBlob) {
        db.execute(stmt.if_not_exists()).await.map_err(|e| {
            AppError::DatabaseError(format!("Failed to create registry_blobs index: {}", e))
        })?;
    }
    Ok(())
}

/// Percent-encode a URL userinfo component so passwords containing URL-special
/// characters (@ / # : etc.) do not corrupt the redis:// connection URL.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub async fn init_redis(config: &RedisConfig) -> Result<redis::aio::ConnectionManager, AppError> {
    let url = if config.password.is_empty() {
        format!("redis://{}:{}", config.host, config.port)
    } else {
        format!(
            "redis://:{}@{}:{}",
            percent_encode(&config.password),
            config.host,
            config.port
        )
    };
    let client = redis::Client::open(url)
        .map_err(|e| AppError::DatabaseError(format!("Failed to create Redis client: {}", e)))?;
    let manager = redis::aio::ConnectionManager::new(client)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Failed to connect to Redis: {}", e)))?;
    let mut conn = manager.clone();
    let _: String = redis::cmd("PING")
        .query_async(&mut conn)
        .await
        .map_err(|e| AppError::DatabaseError(format!("Redis ping failed: {}", e)))?;

    info!("Redis connection established");
    Ok(manager)
}

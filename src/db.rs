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
    opts.max_connections(config.max_connections)
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

pub async fn init_redis(config: &RedisConfig) -> Result<redis::aio::ConnectionManager, AppError> {
    let url = if config.password.is_empty() {
        format!("redis://{}:{}", config.host, config.port)
    } else {
        format!(
            "redis://:{}@{}:{}",
            config.password, config.host, config.port
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

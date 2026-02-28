pub mod api;
pub mod client;
pub mod config;
pub mod crypto;
pub mod db;
pub mod error;
pub mod ingest_engine;
pub mod models;
pub mod updater;
pub mod utils;

use crate::client::SekaiClient;
use crate::config::Config;
use sea_orm::DatabaseConnection;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub clients: std::collections::HashMap<crate::config::ServerRegion, Arc<SekaiClient>>,
    pub db: Option<DatabaseConnection>,
    pub master_db: Option<DatabaseConnection>,
    pub redis: Option<redis::aio::ConnectionManager>,
    pub jwt_secret: Option<String>,
}

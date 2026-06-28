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
use std::collections::HashMap;
use std::sync::Arc;

/// A shared in-flight upstream call: holds the (status, serialized-json) result
/// once the leader resolves it; followers await and clone the Arc<str> cheaply.
pub type CoalescedCell = Arc<tokio::sync::OnceCell<(u16, Arc<str>)>>;

/// In-process single-flight for read-endpoint responses. Concurrent requests for
/// the same cache key share one in-flight upstream call (and its result) instead
/// of each hitting the game server, capping per-key upstream/account usage at ~1
/// per cache window regardless of concurrency.
#[derive(Default)]
pub struct RequestCoalescer {
    pub inflight: parking_lot::Mutex<HashMap<String, CoalescedCell>>,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub clients: HashMap<crate::config::ServerRegion, Arc<SekaiClient>>,
    pub db: Option<DatabaseConnection>,
    pub master_db: Option<DatabaseConnection>,
    pub redis: Option<redis::aio::ConnectionManager>,
    pub jwt_secret: Option<String>,
    pub coalescer: Arc<RequestCoalescer>,
}

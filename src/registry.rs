//! Master data manager ("registry"): the headless role that owns a cluster's
//! master data distribution. It pulls each region's master from the owner
//! node (the account node that produced it), runs git push and DB ingest
//! through the same [`crate::updater::sync::MasterSyncer`] a SekaiAPI peer
//! would, and then publishes a per-region manifest, keeps a publish history,
//! serves files and bundles, holds the app-identity override, and fans out
//! update notices to subscribers. Runs as the `master_registry` binary.

pub mod http;
pub mod metas;
pub mod service;
pub mod state;

pub use service::Registry;

//! Registry service: ties the per-region syncers (pull from owner, ingest,
//! git push) to the on-disk state (manifest, history, app identity) and the
//! subscriber fan-out.

use std::collections::HashMap;
use std::sync::Arc;

use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tracing::{error, info, warn};

use super::metas::MusicMetasManager;
use super::state::RegistryState;
use crate::api::internal::{build_master_manifest, MasterManifest};
use crate::client::helper::AppInfo;
use crate::config::{Config, ServerRegion};
use crate::error::AppError;
use crate::updater::sync::MasterSyncer;

/// Outcome of pushing an app identity to one account node.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppIdentityPush {
    pub url: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub struct Registry {
    pub config: Arc<Config>,
    pub state: RegistryState,
    pub syncers: HashMap<ServerRegion, Arc<MasterSyncer>>,
    /// The music_metas feed, when `registry.music_metas.enabled`.
    pub metas: Option<MusicMetasManager>,
    http: reqwest::Client,
    /// One publish at a time per region so a webhook and a poll cannot
    /// interleave their manifest/history writes.
    publish_locks: HashMap<ServerRegion, tokio::sync::Mutex<()>>,
    /// Serializes app-identity writes and their delivery per region so two
    /// concurrent PUTs cannot persist one identity and deliver the other.
    app_locks: HashMap<ServerRegion, tokio::sync::Mutex<()>>,
}

impl Registry {
    pub fn new(config: Arc<Config>, syncers: HashMap<ServerRegion, Arc<MasterSyncer>>) -> Self {
        let state = RegistryState::new(&config.registry.state_dir);
        let metas = if config.registry.music_metas.enabled {
            match MusicMetasManager::new(&config) {
                Ok(m) => Some(m),
                Err(e) => {
                    error!("music_metas feed disabled: {}", e);
                    None
                }
            }
        } else {
            None
        };
        let publish_locks = config
            .servers
            .keys()
            .map(|region| (*region, tokio::sync::Mutex::new(())))
            .collect();
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self {
            config,
            state,
            syncers,
            metas,
            http,
            publish_locks,
            app_locks: [
                ServerRegion::Jp,
                ServerRegion::En,
                ServerRegion::Tw,
                ServerRegion::Kr,
                ServerRegion::Cn,
            ]
            .into_iter()
            .map(|region| (region, tokio::sync::Mutex::new(())))
            .collect(),
        }
    }

    /// Store a (possibly partial) app-identity override and deliver the
    /// resulting complete identity to every account node, serialized per
    /// region. Omitted fields are filled from the current effective identity
    /// (previous override, else the owner's synced version file); a field
    /// that cannot be filled from anywhere is rejected.
    pub async fn set_app_identity(
        &self,
        region: ServerRegion,
        requested: &AppInfo,
    ) -> Result<(AppInfo, Vec<AppIdentityPush>), AppError> {
        let _guard = match self.app_locks.get(&region) {
            Some(lock) => lock.lock().await,
            None => return Err(AppError::InvalidServerRegion(region.as_str().to_string())),
        };
        let current = self.app_identity(region).await.ok();
        let pick =
            |given: &str, field: &str, fallback: Option<&String>| -> Result<String, AppError> {
                if !given.trim().is_empty() {
                    return Ok(given.trim().to_string());
                }
                fallback
                    .filter(|v| !v.trim().is_empty())
                    .cloned()
                    .ok_or_else(|| {
                        AppError::ParseError(format!(
                            "{field} is required: no current value to keep for region {}",
                            region.as_str()
                        ))
                    })
            };
        let effective = AppInfo {
            app_version: pick(
                &requested.app_version,
                "appVersion",
                current.as_ref().map(|c| &c.app_version),
            )?,
            app_hash: pick(
                &requested.app_hash,
                "appHash",
                current.as_ref().map(|c| &c.app_hash),
            )?,
        };
        self.state.set_app_identity(region, &effective).await?;
        info!(
            "{} App identity override set: appVersion={} appHash={}",
            region.as_str().to_uppercase(),
            effective.app_version,
            effective.app_hash.chars().take(16).collect::<String>()
        );
        let pushed = self.push_app_identity(region, &effective).await;
        Ok((effective, pushed))
    }

    /// Regions this registry manages: those with a master directory.
    pub fn regions(&self) -> Vec<ServerRegion> {
        let mut regions: Vec<_> = self
            .config
            .servers
            .iter()
            .filter(|(_, server)| !server.master_dir.is_empty())
            .map(|(region, _)| *region)
            .collect();
        regions.sort();
        regions
    }

    /// `(master_dir, version_path)` for a managed region.
    pub fn region_paths(&self, region: ServerRegion) -> Result<(String, String), AppError> {
        self.config
            .servers
            .get(&region)
            .filter(|server| !server.master_dir.is_empty())
            .map(|server| (server.master_dir.clone(), server.version_path.clone()))
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "region {} has no master directory configured",
                    region.as_str()
                ))
            })
    }

    /// Effective app identity for a region: the operator override when one
    /// is stored, otherwise what the region's version file (synced from the
    /// owner, i.e. the identity the owner logged in with) says.
    pub async fn app_identity(&self, region: ServerRegion) -> Result<AppInfo, AppError> {
        if let Some(info) = self.state.app_identity(region).await? {
            return Ok(info);
        }
        let (_, version_path) = self.region_paths(region)?;
        if version_path.is_empty() {
            return Err(AppError::NotFound(format!(
                "region {} has no version file configured",
                region.as_str()
            )));
        }
        let data = tokio::fs::read(&version_path).await?;
        let version: crate::client::helper::VersionInfo = sonic_rs::from_slice(&data)
            .map_err(|e| AppError::ParseError(format!("version file: {e}")))?;
        Ok(AppInfo {
            app_version: version.app_version,
            app_hash: version.app_hash,
        })
    }

    /// Build the region's manifest from disk, record it, and notify
    /// subscribers when it changed. Returns the manifest and whether it was
    /// a new publish.
    pub async fn publish(&self, region: ServerRegion) -> Result<(MasterManifest, bool), AppError> {
        let (manifest, changed) = {
            // The lock covers manifest build and state write only; the
            // subscriber fan-out (up to one timeout per unreachable peer)
            // runs after it is released so a concurrent refresh is not held up.
            let _guard = match self.publish_locks.get(&region) {
                Some(lock) => lock.lock().await,
                None => return Err(AppError::InvalidServerRegion(region.as_str().to_string())),
            };
            let (master_dir, version_path) = self.region_paths(region)?;
            let manifest = tokio::task::spawn_blocking(move || {
                build_master_manifest(region, &master_dir, &version_path)
            })
            .await
            .map_err(|e| AppError::Internal(format!("manifest task: {e}")))??;
            let changed = self.state.publish(region, &manifest).await?;
            (manifest, changed)
        };
        if changed {
            info!(
                "{} Published master dataVersion {} ({} files)",
                region.as_str().to_uppercase(),
                manifest.data_version,
                manifest.files.len()
            );
            self.notify_subscribers(region, &manifest.data_version)
                .await;
        }
        Ok((manifest, changed))
    }

    /// Pull the region from its owner if newer, then publish. Returns whether
    /// new master data was applied.
    pub async fn refresh(&self, region: ServerRegion) -> Result<bool, AppError> {
        let Some(syncer) = self.syncers.get(&region) else {
            return Err(AppError::NotFound(format!(
                "no master sync configured for region {}",
                region.as_str()
            )));
        };
        let applied = syncer.sync_once().await?;
        if applied {
            self.publish(region).await?;
        }
        Ok(applied)
    }

    /// Startup pass: publish every managed region whose directory already
    /// holds master files but has no recorded manifest yet, so `current` is
    /// available before the first sync.
    pub async fn publish_missing(&self) {
        for region in self.regions() {
            match self.state.current(region).await {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(e) => {
                    warn!(
                        "{} Unreadable registry state, republishing: {}",
                        region.as_str().to_uppercase(),
                        e
                    );
                }
            }
            match self.publish(region).await {
                Ok(_) => {}
                Err(AppError::IoError(e)) => info!(
                    "{} Not published at startup (no master data yet): {}",
                    region.as_str().to_uppercase(),
                    e
                ),
                Err(e) => error!(
                    "{} Startup publish failed: {}",
                    region.as_str().to_uppercase(),
                    e
                ),
            }
        }
    }

    /// Push an app identity to every configured account node
    /// (`POST /internal/app-identity`), returning one outcome per node.
    pub async fn push_app_identity(
        &self,
        region: ServerRegion,
        info: &AppInfo,
    ) -> Vec<AppIdentityPush> {
        let payload = serde_json::json!({
            "server": region.as_str(),
            "appVersion": info.app_version,
            "appHash": info.app_hash,
        });
        // Bounded fan-out; `buffered` keeps configuration order in the result.
        const PUSH_CONCURRENCY: usize = 4;
        use futures::StreamExt;
        let nodes: Vec<crate::config::MasterSyncPeer> = self.config.registry.account_nodes.clone();
        futures::stream::iter(nodes)
            .map(|node| {
                let payload = payload.clone();
                async move { self.push_app_identity_to(region, &node, payload).await }
            })
            .buffered(PUSH_CONCURRENCY)
            .collect()
            .await
    }

    async fn push_app_identity_to(
        &self,
        region: ServerRegion,
        node: &crate::config::MasterSyncPeer,
        payload: serde_json::Value,
    ) -> AppIdentityPush {
        {
            let endpoint = format!("{}/internal/app-identity", node.url.trim_end_matches('/'));
            let mut req = self.http.post(&endpoint).json(&payload);
            if !node.token.is_empty() {
                req = req.bearer_auth(&node.token);
            }
            let outcome = match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();
                    if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                        info!(
                            "{} App identity pushed to {}",
                            region.as_str().to_uppercase(),
                            node.url
                        );
                        AppIdentityPush {
                            url: node.url.clone(),
                            ok: true,
                            message: None,
                        }
                    } else {
                        let message = body
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("rejected")
                            .to_string();
                        warn!(
                            "{} Account node {} rejected app identity: {}",
                            region.as_str().to_uppercase(),
                            node.url,
                            message
                        );
                        AppIdentityPush {
                            url: node.url.clone(),
                            ok: false,
                            message: Some(message),
                        }
                    }
                }
                Ok(resp) => {
                    warn!(
                        "{} Account node {} returned {}",
                        region.as_str().to_uppercase(),
                        node.url,
                        resp.status()
                    );
                    AppIdentityPush {
                        url: node.url.clone(),
                        ok: false,
                        message: Some(format!("HTTP {}", resp.status())),
                    }
                }
                Err(e) => {
                    warn!(
                        "{} Failed to push app identity to {}: {}",
                        region.as_str().to_uppercase(),
                        node.url,
                        e
                    );
                    AppIdentityPush {
                        url: node.url.clone(),
                        ok: false,
                        message: Some(e.to_string()),
                    }
                }
            };
            outcome
        }
    }

    async fn notify_subscribers(&self, region: ServerRegion, data_version: &str) {
        let payload = serde_json::json!({
            "server": region.as_str(),
            "dataVersion": data_version,
        });
        for peer in &self.config.registry.subscribers {
            let endpoint = format!("{}/internal/master-updated", peer.url.trim_end_matches('/'));
            let mut req = self.http.post(&endpoint).json(&payload);
            if !peer.token.is_empty() {
                req = req.bearer_auth(&peer.token);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => info!(
                    "{} Notified subscriber {}",
                    region.as_str().to_uppercase(),
                    peer.url
                ),
                Ok(resp) => warn!(
                    "{} Subscriber {} returned {}",
                    region.as_str().to_uppercase(),
                    peer.url,
                    resp.status()
                ),
                Err(e) => warn!(
                    "{} Failed to notify subscriber {}: {}",
                    region.as_str().to_uppercase(),
                    peer.url,
                    e
                ),
            }
        }
    }

    /// Schedule the per-region fallback polls (`master_sync.poll_cron`), each
    /// running `refresh`, plus the music_metas tick. Owner webhooks remain the
    /// primary trigger for master data.
    pub async fn start_polls(self: &Arc<Self>) -> Result<JobScheduler, JobSchedulerError> {
        let scheduler = JobScheduler::new().await?;
        if self.metas.is_some() {
            let cron = self.config.registry.music_metas.cron.clone();
            let registry = self.clone();
            // First pull right away so `current` exists before the first tick.
            tokio::spawn({
                let registry = registry.clone();
                async move {
                    if let Some(metas) = &registry.metas {
                        metas.refresh_all().await;
                    }
                }
            });
            match Job::new_async(cron.as_str(), move |_uuid, _lock| {
                let registry = registry.clone();
                Box::pin(async move {
                    if let Some(metas) = &registry.metas {
                        metas.refresh_all().await;
                    }
                })
            }) {
                Ok(job) => {
                    scheduler.add(job).await?;
                    info!("music_metas tick scheduled: {}", cron);
                }
                Err(e) => error!("Invalid music_metas cron '{}': {}", cron, e),
            }
        }
        for (region, syncer) in &self.syncers {
            let _ = syncer;
            let cron = self
                .config
                .servers
                .get(region)
                .map(|s| s.master_sync.poll_cron.clone())
                .unwrap_or_default();
            if cron.is_empty() {
                continue;
            }
            let registry = self.clone();
            let region = *region;
            let job = Job::new_async(cron.as_str(), move |_uuid, _lock| {
                let registry = registry.clone();
                Box::pin(async move {
                    if let Err(e) = registry.refresh(region).await {
                        error!(
                            "{} Registry poll failed: {}",
                            region.as_str().to_uppercase(),
                            e
                        );
                    }
                })
            });
            match job {
                Ok(job) => {
                    scheduler.add(job).await?;
                    info!(
                        "{} Registry poll scheduled: {}",
                        region.as_str().to_uppercase(),
                        cron
                    );
                }
                Err(e) => error!(
                    "{} Invalid poll cron '{}': {}",
                    region.as_str().to_uppercase(),
                    cron,
                    e
                ),
            }
        }
        scheduler.start().await?;
        Ok(scheduler)
    }
}

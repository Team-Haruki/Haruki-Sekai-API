use std::sync::Arc;

use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tracing::{error, info};

use super::apphash::AppHashUpdater;
use super::master::MasterUpdater;
use crate::client::SekaiClient;
use crate::config::{Config, ServerRegion};

const DEFAULT_COOKIE_REFRESH_CRON: &str = "0 0 */20 * * *";

pub async fn start_scheduler(
    clients: &std::collections::HashMap<ServerRegion, Arc<SekaiClient>>,
    config: &Config,
    db: Option<sea_orm::DatabaseConnection>,
    version_locks: &std::collections::HashMap<ServerRegion, Arc<tokio::sync::Mutex<()>>>,
    syncers: &std::collections::HashMap<ServerRegion, Arc<super::sync::MasterSyncer>>,
) -> Result<JobScheduler, JobSchedulerError> {
    let sched = JobScheduler::new().await?;
    let git_config = &config.git;
    let proxy = if config.proxy.is_empty() {
        None
    } else {
        Some(config.proxy.clone())
    };
    for (region, client) in clients {
        if client.config.require_cookies && client.cookie_helper.is_some() {
            let region_name = region.as_str().to_uppercase();
            let client_clone = client.clone();
            info!(
                "{} Cookie refresh scheduled: {}",
                region_name, DEFAULT_COOKIE_REFRESH_CRON
            );

            match Job::new_async(DEFAULT_COOKIE_REFRESH_CRON, move |_uuid, _lock| {
                let client = client_clone.clone();
                let region_str = region_name.clone();
                Box::pin(async move {
                    info!("{} Running scheduled cookie refresh...", region_str);
                    match client.refresh_cookies().await {
                        Ok(()) => {
                            info!("{} Cookies refreshed successfully", region_str);
                        }
                        Err(e) => {
                            error!("{} Failed to refresh cookies: {}", region_str, e);
                        }
                    }
                })
            }) {
                Ok(job) => {
                    if let Err(e) = sched.add(job).await {
                        error!(
                            "{} Failed to add cookie refresh job: {}",
                            region.as_str().to_uppercase(),
                            e
                        );
                    }
                }
                Err(e) => {
                    error!(
                        "{} Invalid cron expression '{}': {}",
                        region.as_str().to_uppercase(),
                        DEFAULT_COOKIE_REFRESH_CRON,
                        e
                    );
                }
            }
        }
    }

    for (region, client) in clients {
        let server_config = &client.config;
        // One lock per region (shared with the master syncer via AppState), so
        // version-file read-modify-writes never clobber each other.
        let version_lock = version_locks
            .get(region)
            .cloned()
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        if server_config.enable_master_updater && !server_config.master_updater_cron.is_empty() {
            let region_name = region.as_str().to_uppercase();
            let cron_expr = server_config.master_updater_cron.clone();
            info!("{} Master updater scheduled: {}", region_name, cron_expr);
            let git_cfg = if git_config.enabled {
                Some(git_config)
            } else {
                None
            };
            let updater = Arc::new(MasterUpdater::new(
                *region,
                client.clone(),
                git_cfg,
                proxy.clone(),
                config.asset_updater_servers.clone(),
                db.clone(),
                version_lock.clone(),
                None,
            ));
            match Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
                let updater = updater.clone();
                Box::pin(async move {
                    updater.check_update().await;
                })
            }) {
                Ok(job) => {
                    if let Err(e) = sched.add(job).await {
                        error!("{} Failed to add master updater job: {}", region_name, e);
                    }
                }
                Err(e) => {
                    error!(
                        "{} Invalid cron expression '{}': {}",
                        region_name, server_config.master_updater_cron, e
                    );
                }
            }
        }
    }

    // App-hash updaters never touch a client (they only read apphash sources
    // and rewrite the version file), so schedule them from config alone: a
    // node that produces master data with remote accounts (no local client)
    // must still keep its version files' appVersion/appHash fresh.
    for (region, server_config) in &config.servers {
        if !server_config.enable_app_hash_updater
            || server_config.app_hash_updater_cron.is_empty()
            || server_config.version_path.is_empty()
        {
            continue;
        }
        let region_name = region.as_str().to_uppercase();
        let cron_expr = server_config.app_hash_updater_cron.clone();
        if config.apphash_sources.is_empty() {
            info!(
                "{} AppHash updater disabled: no sources configured",
                region_name
            );
            continue;
        }
        let version_lock = version_locks
            .get(region)
            .cloned()
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        info!("{} AppHash updater scheduled: {}", region_name, cron_expr);
        let updater = Arc::new(AppHashUpdater::new(
            *region,
            config.apphash_sources.clone(),
            server_config.version_path.clone(),
            proxy.clone(),
            version_lock,
        ));
        match Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
            let updater = updater.clone();
            Box::pin(async move {
                updater.check_update().await;
            })
        }) {
            Ok(job) => {
                if let Err(e) = sched.add(job).await {
                    error!("{} Failed to add apphash updater job: {}", region_name, e);
                }
            }
            Err(e) => {
                error!(
                    "{} Invalid cron expression '{}': {}",
                    region_name, server_config.app_hash_updater_cron, e
                );
            }
        }
    }
    // Remote-source master production: regions whose game accounts live on a
    // peer node but whose master pipeline (download, decode, ingest, git push)
    // runs here on a headless client — the peer only serves the login probe
    // and relays encrypted split bytes. Applies only to regions without a
    // local client; a region with its own accounts uses the classic updater.
    let mut headless_http: Option<reqwest::Client> = None;
    for (region, server_config) in &config.servers {
        let remote_cfg = &server_config.master_remote_source;
        if remote_cfg.url.is_empty()
            || !server_config.enable_master_updater
            || server_config.master_updater_cron.is_empty()
            || clients.contains_key(region)
        {
            continue;
        }
        let region_name = region.as_str().to_uppercase();
        let http = match &headless_http {
            Some(h) => h.clone(),
            None => match SekaiClient::build_http_client(proxy.as_deref()) {
                Ok(h) => {
                    headless_http = Some(h.clone());
                    h
                }
                Err(e) => {
                    error!(
                        "{} Failed to build headless http client: {}",
                        region_name, e
                    );
                    continue;
                }
            },
        };
        let nuverse_store = if !region.is_cp_server()
            && !server_config.nuverse_schema_bundle_path.is_empty()
        {
            match SekaiClient::load_nuverse_schema_store(&server_config.nuverse_schema_bundle_path)
            {
                Ok(store) => Some(Arc::new(store)),
                Err(e) => {
                    error!("{} Failed to load schema bundle: {}", region_name, e);
                    continue;
                }
            }
        } else {
            None
        };
        let client = match SekaiClient::new(
            *region,
            server_config.clone(),
            proxy.clone(),
            None,
            http,
            nuverse_store,
        )
        .await
        {
            Ok(c) => Arc::new(c),
            Err(e) => {
                error!("{} Failed to build headless client: {}", region_name, e);
                continue;
            }
        };
        let bulk_http = match crate::upstream::build_bulk_internal_http_client() {
            Ok(h) => h,
            Err(e) => {
                error!("{} Failed to build bulk http client: {}", region_name, e);
                continue;
            }
        };
        let remote = super::master::RemoteMasterSource::new(*region, remote_cfg, bulk_http);
        let version_lock = version_locks
            .get(region)
            .cloned()
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        let git_cfg = if git_config.enabled {
            Some(git_config)
        } else {
            None
        };
        let cron_expr = server_config.master_updater_cron.clone();
        info!(
            "{} Master updater (remote accounts via {}) scheduled: {}",
            region_name, remote_cfg.url, cron_expr
        );
        let updater = Arc::new(MasterUpdater::new(
            *region,
            client,
            git_cfg,
            proxy.clone(),
            config.asset_updater_servers.clone(),
            db.clone(),
            version_lock,
            Some(remote),
        ));
        match Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
            let updater = updater.clone();
            Box::pin(async move {
                updater.check_update().await;
            })
        }) {
            Ok(job) => {
                if let Err(e) = sched.add(job).await {
                    error!(
                        "{} Failed to add remote master updater job: {}",
                        region_name, e
                    );
                }
            }
            Err(e) => {
                error!(
                    "{} Invalid cron expression '{}': {}",
                    region.as_str().to_uppercase(),
                    cron_expr,
                    e
                );
            }
        }
    }

    // Fallback master-sync polling: webhook from the owner is the fast path;
    // this poll catches missed webhooks and nodes that were down at the time.
    for (region, syncer) in syncers {
        let cron_expr = config
            .servers
            .get(region)
            .map(|c| c.master_sync.poll_cron.clone())
            .unwrap_or_default();
        if cron_expr.is_empty() {
            continue;
        }
        let region_name = region.as_str().to_uppercase();
        info!("{} Master sync poll scheduled: {}", region_name, cron_expr);
        let syncer = syncer.clone();
        match Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
            let syncer = syncer.clone();
            let region_name = region_name.clone();
            Box::pin(async move {
                if let Err(e) = syncer.sync_once().await {
                    error!("{} Master sync poll failed: {}", region_name, e);
                }
            })
        }) {
            Ok(job) => {
                if let Err(e) = sched.add(job).await {
                    error!(
                        "{} Failed to add master sync poll job: {}",
                        region.as_str().to_uppercase(),
                        e
                    );
                }
            }
            Err(e) => {
                error!(
                    "{} Invalid cron expression '{}': {}",
                    region.as_str().to_uppercase(),
                    cron_expr,
                    e
                );
            }
        }
    }

    sched.start().await?;
    info!("Scheduler started");
    Ok(sched)
}

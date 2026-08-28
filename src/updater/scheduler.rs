use std::collections::HashMap;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tracing::{error, info};

use super::apphash::AppHashUpdater;
use super::master::{MasterUpdater, RemoteMasterSource};
use super::sync::MasterSyncer;
use crate::client::SekaiClient;
use crate::config::{Config, ServerRegion};

const DEFAULT_COOKIE_REFRESH_CRON: &str = "0 0 */20 * * *";

type ClientMap = HashMap<ServerRegion, Arc<SekaiClient>>;
type VersionLocks = HashMap<ServerRegion, Arc<tokio::sync::Mutex<()>>>;
type SyncerMap = HashMap<ServerRegion, Arc<MasterSyncer>>;

pub async fn start_scheduler(
    clients: &ClientMap,
    config: &Config,
    db: Option<DatabaseConnection>,
    version_locks: &VersionLocks,
    syncers: &SyncerMap,
) -> Result<JobScheduler, JobSchedulerError> {
    let scheduler = JobScheduler::new().await?;
    let proxy = (!config.proxy.is_empty()).then(|| config.proxy.clone());
    schedule_cookie_refreshes(&scheduler, clients).await;
    schedule_local_master_updates(
        &scheduler,
        clients,
        config,
        db.clone(),
        version_locks,
        &proxy,
    )
    .await;
    schedule_apphash_updates(&scheduler, config, version_locks, &proxy).await;
    schedule_remote_master_updates(&scheduler, clients, config, db, version_locks, &proxy).await;
    schedule_master_syncs(&scheduler, config, syncers).await;
    scheduler.start().await?;
    info!("Scheduler started");
    Ok(scheduler)
}

async fn add_job(
    scheduler: &JobScheduler,
    region: ServerRegion,
    cron: &str,
    label: &str,
    job: Result<Job, JobSchedulerError>,
) {
    let region = region.as_str().to_uppercase();
    match job {
        Ok(job) => {
            if let Err(e) = scheduler.add(job).await {
                error!("{} Failed to add {} job: {}", region, label, e);
            }
        }
        Err(e) => error!("{} Invalid cron expression '{}': {}", region, cron, e),
    }
}

async fn schedule_cookie_refreshes(scheduler: &JobScheduler, clients: &ClientMap) {
    for (region, client) in clients {
        if !client.config.require_cookies || client.cookie_helper.is_none() {
            continue;
        }
        let region_name = region.as_str().to_uppercase();
        let client = client.clone();
        info!(
            "{} Cookie refresh scheduled: {}",
            region_name, DEFAULT_COOKIE_REFRESH_CRON
        );
        let job = Job::new_async(DEFAULT_COOKIE_REFRESH_CRON, move |_uuid, _lock| {
            let client = client.clone();
            let region = region_name.clone();
            Box::pin(async move {
                info!("{} Running scheduled cookie refresh...", region);
                match client.refresh_cookies().await {
                    Ok(()) => info!("{} Cookies refreshed successfully", region),
                    Err(e) => error!("{} Failed to refresh cookies: {}", region, e),
                }
            })
        });
        add_job(
            scheduler,
            *region,
            DEFAULT_COOKIE_REFRESH_CRON,
            "cookie refresh",
            job,
        )
        .await;
    }
}

async fn schedule_local_master_updates(
    scheduler: &JobScheduler,
    clients: &ClientMap,
    config: &Config,
    db: Option<DatabaseConnection>,
    version_locks: &VersionLocks,
    proxy: &Option<String>,
) {
    for (region, client) in clients {
        let server = &client.config;
        if !server.enable_master_updater || server.master_updater_cron.is_empty() {
            continue;
        }
        let cron = server.master_updater_cron.clone();
        let updater = Arc::new(MasterUpdater::new(
            *region,
            client.clone(),
            config.git.enabled.then_some(&config.git),
            proxy.clone(),
            config.asset_updater_servers.clone(),
            db.clone(),
            version_lock(version_locks, *region),
            None,
        ));
        info!(
            "{} Master updater scheduled: {}",
            region.as_str().to_uppercase(),
            cron
        );
        let job = Job::new_async(cron.as_str(), move |_uuid, _lock| {
            let updater = updater.clone();
            Box::pin(async move { updater.check_update().await })
        });
        add_job(scheduler, *region, &cron, "master updater", job).await;
    }
}

async fn schedule_apphash_updates(
    scheduler: &JobScheduler,
    config: &Config,
    version_locks: &VersionLocks,
    proxy: &Option<String>,
) {
    for (region, server) in &config.servers {
        if !apphash_enabled(server) {
            continue;
        }
        if config.apphash_sources.is_empty() {
            info!(
                "{} AppHash updater disabled: no sources configured",
                region.as_str().to_uppercase()
            );
            continue;
        }
        let cron = server.app_hash_updater_cron.clone();
        let updater = Arc::new(AppHashUpdater::new(
            *region,
            config.apphash_sources.clone(),
            server.version_path.clone(),
            proxy.clone(),
            version_lock(version_locks, *region),
        ));
        info!(
            "{} AppHash updater scheduled: {}",
            region.as_str().to_uppercase(),
            cron
        );
        let job = Job::new_async(cron.as_str(), move |_uuid, _lock| {
            let updater = updater.clone();
            Box::pin(async move { updater.check_update().await })
        });
        add_job(scheduler, *region, &cron, "apphash updater", job).await;
    }
}

fn apphash_enabled(server: &crate::config::ServerConfig) -> bool {
    server.enable_app_hash_updater
        && !server.app_hash_updater_cron.is_empty()
        && !server.version_path.is_empty()
}

async fn schedule_remote_master_updates(
    scheduler: &JobScheduler,
    clients: &ClientMap,
    config: &Config,
    db: Option<DatabaseConnection>,
    version_locks: &VersionLocks,
    proxy: &Option<String>,
) {
    let mut shared_http = None;
    for (region, server) in &config.servers {
        if !remote_master_enabled(*region, server, clients) {
            continue;
        }
        let Some(client) = build_headless_client(*region, server, proxy, &mut shared_http).await
        else {
            continue;
        };
        let Some(remote) = build_remote_source(*region, &server.master_remote_source) else {
            continue;
        };
        let cron = server.master_updater_cron.clone();
        info!(
            "{} Master updater (remote accounts via {}) scheduled: {}",
            region.as_str().to_uppercase(),
            server.master_remote_source.url,
            cron
        );
        let updater = Arc::new(MasterUpdater::new(
            *region,
            client,
            config.git.enabled.then_some(&config.git),
            proxy.clone(),
            config.asset_updater_servers.clone(),
            db.clone(),
            version_lock(version_locks, *region),
            Some(remote),
        ));
        let job = Job::new_async(cron.as_str(), move |_uuid, _lock| {
            let updater = updater.clone();
            Box::pin(async move { updater.check_update().await })
        });
        add_job(scheduler, *region, &cron, "remote master updater", job).await;
    }
}

fn remote_master_enabled(
    region: ServerRegion,
    server: &crate::config::ServerConfig,
    clients: &ClientMap,
) -> bool {
    !server.master_remote_source.url.is_empty()
        && server.enable_master_updater
        && !server.master_updater_cron.is_empty()
        && !clients.contains_key(&region)
}

async fn build_headless_client(
    region: ServerRegion,
    server: &crate::config::ServerConfig,
    proxy: &Option<String>,
    shared_http: &mut Option<reqwest::Client>,
) -> Option<Arc<SekaiClient>> {
    let http = match shared_http.clone() {
        Some(http) => http,
        None => {
            let http = SekaiClient::build_http_client(proxy.as_deref())
                .map_err(|e| {
                    error!(
                        "{} Failed to build headless http client: {}",
                        region.as_str().to_uppercase(),
                        e
                    );
                })
                .ok()?;
            *shared_http = Some(http.clone());
            http
        }
    };
    let schema = load_headless_schema(region, server)?;
    SekaiClient::new(region, server.clone(), proxy.clone(), None, http, schema)
        .await
        .map(Arc::new)
        .map_err(|e| {
            error!(
                "{} Failed to build headless client: {}",
                region.as_str().to_uppercase(),
                e
            );
        })
        .ok()
}

fn load_headless_schema(
    region: ServerRegion,
    server: &crate::config::ServerConfig,
) -> Option<Option<Arc<crate::client::nuverse_schema::NuverseSchemaStore>>> {
    if region.is_cp_server() || server.nuverse_schema_bundle_path.is_empty() {
        return Some(None);
    }
    SekaiClient::load_nuverse_schema_store(&server.nuverse_schema_bundle_path)
        .map(|schema| Some(Arc::new(schema)))
        .map_err(|e| {
            error!(
                "{} Failed to load schema bundle: {}",
                region.as_str().to_uppercase(),
                e
            );
        })
        .ok()
}

fn build_remote_source(
    region: ServerRegion,
    config: &crate::config::MasterRemoteSourceConfig,
) -> Option<RemoteMasterSource> {
    crate::upstream::build_bulk_internal_http_client()
        .map(|http| RemoteMasterSource::new(region, config, http))
        .map_err(|e| {
            error!(
                "{} Failed to build bulk http client: {}",
                region.as_str().to_uppercase(),
                e
            );
        })
        .ok()
}

async fn schedule_master_syncs(scheduler: &JobScheduler, config: &Config, syncers: &SyncerMap) {
    for (region, syncer) in syncers {
        let cron = config
            .servers
            .get(region)
            .map(|server| server.master_sync.poll_cron.clone())
            .unwrap_or_default();
        if cron.is_empty() {
            continue;
        }
        let region_name = region.as_str().to_uppercase();
        let syncer = syncer.clone();
        info!("{} Master sync poll scheduled: {}", region_name, cron);
        let job = Job::new_async(cron.as_str(), move |_uuid, _lock| {
            let syncer = syncer.clone();
            let region = region_name.clone();
            Box::pin(async move {
                if let Err(e) = syncer.sync_once().await {
                    error!("{} Master sync poll failed: {}", region, e);
                }
            })
        });
        add_job(scheduler, *region, &cron, "master sync poll", job).await;
    }
}

fn version_lock(locks: &VersionLocks, region: ServerRegion) -> Arc<tokio::sync::Mutex<()>> {
    locks
        .get(&region)
        .cloned()
        .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())))
}

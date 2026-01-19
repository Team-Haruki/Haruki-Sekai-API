use std::sync::Arc;

use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tracing::{error, info};

use super::apphash::AppHashUpdater;
use super::master::MasterUpdater;
use crate::client::SekaiClient;
use crate::config::{Config, ServerRegion};

pub async fn start_scheduler(
    clients: &std::collections::HashMap<ServerRegion, Arc<SekaiClient>>,
    config: &Config,
) -> Result<JobScheduler, JobSchedulerError> {
    let sched = JobScheduler::new().await?;
    let git_config = &config.git;
    let proxy = if config.proxy.is_empty() {
        None
    } else {
        Some(config.proxy.clone())
    };
    for (region, client) in clients {
        let server_config = &client.config;
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
        if server_config.enable_app_hash_updater && !server_config.app_hash_updater_cron.is_empty()
        {
            let region_name = region.as_str().to_uppercase();
            let cron_expr = server_config.app_hash_updater_cron.clone();
            if config.apphash_sources.is_empty() {
                info!(
                    "{} AppHash updater disabled: no sources configured",
                    region_name
                );
                continue;
            }
            info!("{} AppHash updater scheduled: {}", region_name, cron_expr);
            let updater = Arc::new(AppHashUpdater::new(
                *region,
                config.apphash_sources.clone(),
                server_config.version_path.clone(),
                proxy.clone(),
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
    }
    sched.start().await?;
    info!("Scheduler started");
    Ok(sched)
}

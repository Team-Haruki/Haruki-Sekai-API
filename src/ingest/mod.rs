//! Registry-driven master ingest (the `master_ingest` binary): reads the
//! registry's `current` and `blob/{sha256}` over HTTP and brings one or more
//! database targets to that version, writing only changed files. See
//! `docs/master-registry-storage-and-ingest.md`, part B.
//!
//! Triggers: a reconcile of every region at startup and on
//! `ingest.reconcile_cron`, and of one region on the registry's publish
//! webhook. A trigger never chooses the data: every run reads `current`.

pub mod http;
pub mod registry_client;
pub mod run;
pub mod target;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Result};
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, warn};

use crate::config::{IngestConfig, ServerRegion};
pub use registry_client::RegistryClient;
pub use run::TargetOutcome;
pub use target::Target;

/// Coalesces triggers of one region: a trigger during a run schedules
/// exactly one follow-up run.
#[derive(Default)]
struct RegionTrigger {
    running: AtomicBool,
    pending: AtomicBool,
}

/// Last outcome per (target, region), for `/health`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusEntry {
    pub at: String,
    #[serde(flatten)]
    pub outcome: TargetOutcome,
}

pub struct Ingester {
    pub config: IngestConfig,
    pub registry: RegistryClient,
    pub targets: Vec<Arc<Target>>,
    regions: Vec<ServerRegion>,
    triggers: HashMap<ServerRegion, RegionTrigger>,
    /// Regions reconciled at once (`parse_concurrency`).
    slots: tokio::sync::Semaphore,
    status: parking_lot::Mutex<BTreeMap<String, BTreeMap<String, StatusEntry>>>,
}

/// Validate the section (names, cron, concurrency) before anything connects.
pub fn validate(config: &IngestConfig) -> Result<()> {
    if config.registry_url.trim().is_empty() {
        bail!("ingest.registry_url is empty");
    }
    if config.targets.is_empty() {
        bail!("ingest.targets is empty");
    }
    if config.parse_concurrency == 0 {
        bail!("ingest.parse_concurrency must be at least 1");
    }
    let mut names = std::collections::HashSet::new();
    for t in &config.targets {
        let valid = !t.name.is_empty()
            && t.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !valid {
            bail!("ingest target name {:?} must be [A-Za-z0-9_-]+", t.name);
        }
        if !names.insert(t.name.as_str()) {
            bail!("ingest target name {:?} is used twice", t.name);
        }
        if t.dsn.trim().is_empty() {
            bail!("ingest target {}: dsn is empty", t.name);
        }
    }
    Ok(())
}

impl Ingester {
    /// Load every target (schema, table lists) and connect lazily.
    pub async fn new(config: IngestConfig) -> Result<Self> {
        validate(&config)?;
        let pool_size_floor = (2 * config.parse_concurrency + 1) as u32;
        let mut targets = Vec::new();
        for cfg in &config.targets {
            let size = if cfg.max_connections < pool_size_floor {
                warn!(
                    "Ingest target {}: max_connections {} raised to {} (2 per concurrent region + 1)",
                    cfg.name, cfg.max_connections, pool_size_floor
                );
                pool_size_floor
            } else {
                cfg.max_connections
            };
            targets.push(Arc::new(Target::open(cfg.clone(), size).await?));
        }
        Ok(Self::with_targets(config, targets))
    }

    pub fn with_targets(config: IngestConfig, targets: Vec<Arc<Target>>) -> Self {
        let mut regions: Vec<ServerRegion> = targets
            .iter()
            .flat_map(|t| t.cfg.regions.iter().copied())
            .collect();
        regions.sort();
        regions.dedup();
        let triggers = regions
            .iter()
            .map(|r| (*r, RegionTrigger::default()))
            .collect();
        Self {
            registry: RegistryClient::new(&config.registry_url),
            slots: tokio::sync::Semaphore::new(config.parse_concurrency.max(1)),
            config,
            targets,
            regions,
            triggers,
            status: parking_lot::Mutex::new(BTreeMap::new()),
        }
    }

    pub fn regions(&self) -> &[ServerRegion] {
        &self.regions
    }

    pub fn target(&self, name: &str) -> Option<&Arc<Target>> {
        self.targets.iter().find(|t| t.name == name)
    }

    /// Wait until the registry answers `/health` and refuse an fs registry
    /// (its `blob/` URLs die with every publish).
    pub async fn check_registry(&self) -> Result<()> {
        let mut delay = std::time::Duration::from_secs(1);
        loop {
            match self.registry.blob_store().await {
                Ok(kind) if kind == "pg" => return Ok(()),
                Ok(kind) => bail!(
                    "registry {} runs blob_store {kind:?}; the ingester requires `registry.blob_store: pg`",
                    self.registry.base()
                ),
                Err(e) => {
                    warn!(
                        "Registry {} not reachable yet ({:#}); retrying in {:?}",
                        self.registry.base(),
                        e,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(std::time::Duration::from_secs(30));
                }
            }
        }
    }

    /// Reconcile one region now (all targets), waiting for a slot.
    pub async fn reconcile_region(&self, region: ServerRegion) -> Vec<(String, TargetOutcome)> {
        let _slot = self.slots.acquire().await;
        let outcomes = run::reconcile_region(
            &self.registry,
            &self.targets,
            region,
            self.config.stage_tables,
            self.config.stage_bytes,
        )
        .await;
        let at = chrono::Utc::now().to_rfc3339();
        let mut status = self.status.lock();
        for (target, outcome) in &outcomes {
            if let TargetOutcome::Skipped { reason } = outcome {
                warn!(
                    "{} Target {} skipped: {}",
                    region.as_str().to_uppercase(),
                    target,
                    reason
                );
            }
            status.entry(target.clone()).or_default().insert(
                region.as_str().to_string(),
                StatusEntry {
                    at: at.clone(),
                    outcome: outcome.clone(),
                },
            );
        }
        outcomes
    }

    /// Schedule a reconcile of `region` in the background (coalesced).
    pub fn trigger(self: &Arc<Self>, region: ServerRegion) -> bool {
        let Some(trigger) = self.triggers.get(&region) else {
            return false;
        };
        trigger.pending.store(true, Ordering::SeqCst);
        if trigger.running.swap(true, Ordering::SeqCst) {
            return true;
        }
        let this = self.clone();
        tokio::spawn(async move {
            let trigger = &this.triggers[&region];
            loop {
                trigger.pending.store(false, Ordering::SeqCst);
                this.reconcile_region(region).await;
                if trigger.pending.load(Ordering::SeqCst) {
                    continue;
                }
                trigger.running.store(false, Ordering::SeqCst);
                // A trigger that saw `running` just before it was cleared.
                if trigger.pending.load(Ordering::SeqCst)
                    && !trigger.running.swap(true, Ordering::SeqCst)
                {
                    continue;
                }
                break;
            }
        });
        true
    }

    pub fn trigger_all(self: &Arc<Self>) {
        for region in self.regions.clone() {
            self.trigger(region);
        }
    }

    /// Timer reconcile on `reconcile_cron` (empty disables it).
    pub async fn start_timer(self: &Arc<Self>) -> Result<Option<JobScheduler>> {
        let cron = self.config.reconcile_cron.trim().to_string();
        if cron.is_empty() {
            return Ok(None);
        }
        let scheduler = JobScheduler::new().await?;
        let this = self.clone();
        let job = Job::new_async(cron.as_str(), move |_uuid, _lock| {
            let this = this.clone();
            Box::pin(async move {
                this.trigger_all();
            })
        })?;
        scheduler.add(job).await?;
        scheduler.start().await?;
        info!("Ingest reconcile scheduled ({cron})");
        Ok(Some(scheduler))
    }

    pub fn status(&self) -> BTreeMap<String, BTreeMap<String, StatusEntry>> {
        self.status.lock().clone()
    }
}

/// Log a startup failure of the ingest role and exit non-zero.
pub fn fatal(e: &anyhow::Error) -> ! {
    error!("master_ingest: {e:#}");
    std::process::exit(1)
}

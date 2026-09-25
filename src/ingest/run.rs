//! One region reconcile across all targets: diff each target's state against
//! the registry's `current`, fetch and parse every changed file once, fan the
//! row batches out to the targets that need them, and let each target commit
//! (or fail) on its own.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use sea_orm::{DatabaseTransaction, TransactionTrait};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use super::registry_client::{Gone, RegistryClient};
use super::target::{write_file, RegionLock, StateRow, TablePlan, Target, VersionRow};
use crate::api::internal::{MasterManifest, MasterManifestFile};
use crate::client::helper::compare_version;
use crate::config::ServerRegion;
use crate::ingest_engine::{is_legacy_skipped_table, stream_rows, CHANNEL_DEPTH, ROWS_PER_BATCH};

/// Every receiver of a file went away (each target that needed it already
/// failed): not a problem with the blob, the other targets carry on.
#[derive(Debug)]
struct NoReceivers;

impl std::fmt::Display for NoReceivers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no target is receiving this file")
    }
}

impl std::error::Error for NoReceivers {}

/// Restarts of a run whose blobs vanished (the registry moved on).
const MAX_RESTARTS: usize = 3;

/// What happened to one target in one region run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum TargetOutcome {
    /// Already at the registry's `current` with no mapping change.
    UpToDate {
        content_hash: String,
    },
    Ingested {
        content_hash: String,
        data_version: String,
        written: usize,
        removed: usize,
        staged: bool,
    },
    /// Another ingester holds the (target, region) lock.
    Busy,
    Skipped {
        reason: String,
    },
    Failed {
        error: String,
    },
}

/// A changed file a target writes.
struct WriteJob {
    file: MasterManifestFile,
    plan: TablePlan,
    prev_rows: Option<i64>,
}

/// A target that has work in this run.
struct Prepared {
    target: Arc<Target>,
    _lock: RegionLock,
    writes: Vec<WriteJob>,
    /// State rows of files that are not written (no table, not selected).
    skips: Vec<(MasterManifestFile, Option<String>, String)>,
    /// Files present again after being recorded missing.
    returned: Vec<String>,
    removed: Vec<(String, StateRow)>,
    /// Manifest files per category, for the file-count check.
    counted: usize,
    staged: bool,
    table_has_region: HashMap<String, bool>,
}

enum Prep {
    Work(Box<Prepared>),
    Done(TargetOutcome),
}

/// Messages from the driver to a target's writer task.
enum Msg {
    File {
        name: String,
        batches: mpsc::Receiver<Arc<Vec<Value>>>,
        parsed: oneshot::Receiver<Result<(), String>>,
    },
    /// Every file was sent: run the checks and commit.
    Finish,
    /// Roll back without recording anything (the run restarts).
    Abandon,
    /// Roll back and record the failure.
    Fail(String),
}

enum TaskEnd {
    Outcome(TargetOutcome),
    Abandoned,
}

pub(crate) async fn reconcile_region(
    registry: &RegistryClient,
    targets: &[Arc<Target>],
    region: ServerRegion,
    stage_tables: usize,
    stage_bytes: u64,
) -> Vec<(String, TargetOutcome)> {
    let serving: Vec<Arc<Target>> = targets
        .iter()
        .filter(|t| t.serves(region))
        .cloned()
        .collect();
    let all = |outcome: TargetOutcome| {
        serving
            .iter()
            .map(|t| (t.name.clone(), outcome.clone()))
            .collect::<Vec<_>>()
    };
    match registry.blob_store().await {
        Ok(kind) if kind == "pg" => {}
        Ok(kind) => {
            return all(TargetOutcome::Skipped {
                reason: format!("registry blob store is {kind:?}; the ingester requires pg"),
            })
        }
        Err(e) => {
            return all(TargetOutcome::Skipped {
                reason: format!("registry unavailable: {e:#}"),
            })
        }
    }
    let mut restarts = 0;
    loop {
        let manifest = match registry.current(region).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                return all(TargetOutcome::Skipped {
                    reason: "region not published".into(),
                })
            }
            Err(e) => {
                return all(TargetOutcome::Skipped {
                    reason: format!("registry unavailable: {e:#}"),
                })
            }
        };
        let manifest = Arc::new(manifest);
        match run_once(
            registry,
            &serving,
            region,
            &manifest,
            stage_tables,
            stage_bytes,
        )
        .await
        {
            Some(outcomes) => return outcomes,
            None if restarts < MAX_RESTARTS => {
                restarts += 1;
                info!(
                    "{} Registry moved past contentHash {}; restarting from the new current",
                    region.as_str().to_uppercase(),
                    manifest.content_hash
                );
            }
            None => {
                return all(TargetOutcome::Skipped {
                    reason: "registry kept moving; retrying on the next trigger".into(),
                })
            }
        }
    }
}

/// One attempt at `manifest`; `None` when it must restart from a newer
/// `current` (a blob answered 404).
async fn run_once(
    registry: &RegistryClient,
    targets: &[Arc<Target>],
    region: ServerRegion,
    manifest: &Arc<MasterManifest>,
    stage_tables: usize,
    stage_bytes: u64,
) -> Option<Vec<(String, TargetOutcome)>> {
    let started_at = chrono::Utc::now();
    let mut outcomes: Vec<(String, TargetOutcome)> = Vec::new();
    let mut prepared: Vec<Prepared> = Vec::new();
    for target in targets {
        match prepare(target.clone(), region, manifest, stage_tables, stage_bytes).await {
            Ok(Prep::Work(p)) => prepared.push(*p),
            Ok(Prep::Done(outcome)) => outcomes.push((target.name.clone(), outcome)),
            Err(e) => {
                let error = format!("{e:#}");
                warn!(
                    "{} Target {} could not be prepared: {}",
                    region.as_str().to_uppercase(),
                    target.name,
                    error
                );
                let _ = target.record_failure(region, &error).await;
                outcomes.push((target.name.clone(), TargetOutcome::Failed { error }));
            }
        }
    }
    if prepared.is_empty() {
        return Some(outcomes);
    }

    // Files to fetch: the union over targets, each with the targets needing it.
    let mut needed: BTreeMap<String, (MasterManifestFile, Vec<usize>)> = BTreeMap::new();
    for (i, p) in prepared.iter().enumerate() {
        for job in &p.writes {
            needed
                .entry(job.file.name.clone())
                .or_insert_with(|| (job.file.clone(), Vec::new()))
                .1
                .push(i);
        }
    }

    let mut controls: Vec<Option<mpsc::Sender<Msg>>> = Vec::new();
    let mut tasks = Vec::new();
    for p in prepared {
        let (tx, rx) = mpsc::channel::<Msg>(1);
        controls.push(Some(tx));
        let name = p.target.name.clone();
        let manifest = manifest.clone();
        tasks.push((
            name,
            tokio::spawn(target_task(p, region, manifest, started_at, rx)),
        ));
    }

    let mut abandoned = false;
    'files: for (name, (file, users)) in needed {
        let live: Vec<usize> = users
            .into_iter()
            .filter(|i| controls[*i].as_ref().is_some_and(|c| !c.is_closed()))
            .collect();
        if live.is_empty() {
            continue;
        }
        let reader = match registry.open_blob(region, &file.sha256, file.size).await {
            Ok(r) => r,
            Err(e) if e.downcast_ref::<Gone>().is_some() => {
                abandoned = true;
                break 'files;
            }
            Err(e) => {
                let error = format!("fetching {name}: {e:#}");
                for c in controls.iter_mut() {
                    if let Some(tx) = c.take() {
                        let _ = tx.send(Msg::Fail(error.clone())).await;
                    }
                }
                break 'files;
            }
        };
        let mut senders = Vec::new();
        let mut outcome_txs = Vec::new();
        for i in live {
            let (btx, brx) = mpsc::channel(CHANNEL_DEPTH);
            let (otx, orx) = oneshot::channel();
            let sent = match &controls[i] {
                Some(tx) => tx
                    .send(Msg::File {
                        name: name.clone(),
                        batches: brx,
                        parsed: orx,
                    })
                    .await
                    .is_ok(),
                None => false,
            };
            if sent {
                senders.push(btx);
                outcome_txs.push(otx);
            } else {
                controls[i] = None;
            }
        }
        let parsed = tokio::task::spawn_blocking(move || parse_fan_out(reader, senders))
            .await
            .map_err(|e| anyhow!("parse task: {e}"))
            .and_then(|r| r);
        let message = parsed
            .as_ref()
            .map(|_| ())
            .map_err(|e| format!("parsing {name}: {e:#}"));
        for otx in outcome_txs {
            let _ = otx.send(message.clone());
        }
        if parsed
            .as_ref()
            .err()
            .is_some_and(|e| e.downcast_ref::<NoReceivers>().is_some())
        {
            // Only failed targets needed this file; they are already gone.
            continue;
        }
        if message.is_err() {
            // The blob itself is bad or truncated: nothing else of this run
            // can commit consistently.
            for c in controls.iter_mut() {
                c.take();
            }
            break 'files;
        }
    }
    for c in controls.iter_mut() {
        if let Some(tx) = c.take() {
            let _ = tx
                .send(if abandoned { Msg::Abandon } else { Msg::Finish })
                .await;
        }
    }
    let mut any_abandoned = false;
    for (name, task) in tasks {
        match task.await {
            Ok(TaskEnd::Outcome(o)) => outcomes.push((name, o)),
            Ok(TaskEnd::Abandoned) => any_abandoned = true,
            Err(e) => outcomes.push((
                name,
                TargetOutcome::Failed {
                    error: format!("writer task: {e}"),
                },
            )),
        }
    }
    if abandoned && any_abandoned {
        return None;
    }
    Some(outcomes)
}

/// Parse a blob once and hand every batch to each live receiver. A receiver
/// that went away (its target failed) is dropped; the others continue.
fn parse_fan_out(
    reader: super::registry_client::BlobReader,
    senders: Vec<mpsc::Sender<Arc<Vec<Value>>>>,
) -> Result<()> {
    let mut senders: Vec<Option<mpsc::Sender<Arc<Vec<Value>>>>> =
        senders.into_iter().map(Some).collect();
    let reader = std::io::BufReader::with_capacity(256 * 1024, reader);
    let mut no_receivers = false;
    let parsed = stream_rows(reader, ROWS_PER_BATCH, |rows| {
        let batch = Arc::new(rows);
        let mut live = 0;
        for slot in senders.iter_mut() {
            if let Some(tx) = slot {
                if tx.blocking_send(batch.clone()).is_ok() {
                    live += 1;
                } else {
                    *slot = None;
                }
            }
        }
        if live == 0 {
            no_receivers = true;
            bail!(NoReceivers);
        }
        Ok(())
    });
    match parsed {
        Err(_) if no_receivers => Err(NoReceivers.into()),
        other => other,
    }
}

async fn prepare(
    target: Arc<Target>,
    region: ServerRegion,
    manifest: &MasterManifest,
    stage_tables: usize,
    stage_bytes: u64,
) -> Result<Prep> {
    target.ensure_state_tables().await?;
    let Some(lock) = target.try_lock(region).await? else {
        return Ok(Prep::Done(TargetOutcome::Busy));
    };
    let version = target.load_version(&target.db, region, false).await?;
    if let Some(recorded) = version.as_ref().and_then(|v| v.data_version.as_deref()) {
        if is_newer(recorded, &manifest.data_version) {
            return Ok(Prep::Done(TargetOutcome::Skipped {
                reason: format!(
                    "target is at dataVersion {recorded}, newer than the registry's {}",
                    manifest.data_version
                ),
            }));
        }
    }

    let mut files: Vec<&MasterManifestFile> = manifest.files.iter().collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    // file -> table it maps to (legacy-skipped tables count as unmapped).
    let resolved: Vec<(&MasterManifestFile, Option<String>)> = files
        .iter()
        .map(|f| {
            let stem = f.name.strip_suffix(".json").unwrap_or(&f.name);
            let table = target
                .schema
                .resolve_table_name(stem)
                .filter(|t| !is_legacy_skipped_table(t));
            (*f, table)
        })
        .collect();
    let wanted: Vec<String> = resolved
        .iter()
        .filter_map(|(_, t)| t.clone())
        .filter(|t| target.is_selected(t))
        .chain(target.required().iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut shapes = target.introspect().await?;
    if target.cfg.create_tables {
        let missing: Vec<String> = wanted
            .iter()
            .filter(|t| !shapes.contains_key(*t))
            .cloned()
            .collect();
        if !missing.is_empty() {
            target.create_missing_tables(&missing).await?;
            shapes = target.introspect().await?;
        }
    }
    let state = target.load_state(&target.db, region).await?;

    let mut seen_tables: HashMap<String, String> = HashMap::new();
    let mut writes = Vec::new();
    let mut skips = Vec::new();
    let mut returned = Vec::new();
    let mut table_has_region = HashMap::new();
    let mut unchanged = 0usize;
    for (file, table) in &resolved {
        let prev = state.get(&file.name);
        if prev.is_some_and(|p| p.missing_since.is_some()) {
            returned.push(file.name.clone());
        }
        let (table, reason) = match table {
            None => (None, "unmapped"),
            Some(t) if !target.is_selected(t) => (Some(t.as_str()), "not-selected"),
            Some(t) => (Some(t.as_str()), ""),
        };
        if !reason.is_empty() {
            let hash = Target::skip_hash(table, reason);
            if prev.is_none_or(|p| p.sha256 != file.sha256 || p.mapping_hash != hash) {
                skips.push(((*file).clone(), None, hash));
            } else {
                unchanged += 1;
            }
            continue;
        }
        let table = table.unwrap_or_default().to_string();
        if let Some(other) = seen_tables.insert(table.clone(), file.name.clone()) {
            bail!("files {other} and {} both map to table {table}", file.name);
        }
        let shape = shapes.get(&table).ok_or_else(|| {
            anyhow!("table {table} does not exist in the target database (create_tables is off)")
        })?;
        let plan = target.plan(&table, shape)?;
        table_has_region.insert(table.clone(), plan.has_region);
        if prev.is_some_and(|p| p.sha256 == file.sha256 && p.mapping_hash == plan.mapping_hash) {
            unchanged += 1;
            continue;
        }
        writes.push(WriteJob {
            file: (*file).clone(),
            plan,
            prev_rows: prev.and_then(|p| p.rows),
        });
    }
    let listed: HashSet<&str> = manifest.files.iter().map(|f| f.name.as_str()).collect();
    let mut removed: Vec<(String, StateRow)> = state
        .into_iter()
        .filter(|(f, _)| !listed.contains(f.as_str()))
        .collect();
    removed.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, row) in &removed {
        if let Some(t) = &row.table_name {
            if let Some(shape) = shapes.get(t) {
                table_has_region.insert(t.clone(), shape.columns.contains_key("server_region"));
            }
        }
    }
    for t in target.required() {
        if let Some(shape) = shapes.get(t) {
            table_has_region
                .entry(t.clone())
                .or_insert_with(|| shape.columns.contains_key("server_region"));
        }
    }

    let at_version = version.as_ref().is_some_and(|v: &VersionRow| {
        v.status == "ok" && v.content_hash.as_deref() == Some(manifest.content_hash.as_str())
    });
    if writes.is_empty()
        && skips.is_empty()
        && removed.is_empty()
        && returned.is_empty()
        && at_version
    {
        return Ok(Prep::Done(TargetOutcome::UpToDate {
            content_hash: manifest.content_hash.clone(),
        }));
    }
    let bytes: u64 = writes.iter().map(|w| w.file.size).sum();
    let staged = writes.len() > stage_tables || bytes > stage_bytes;
    let counted = writes.len() + skips.len() + unchanged;
    Ok(Prep::Work(Box::new(Prepared {
        target,
        _lock: lock,
        writes,
        skips,
        returned,
        removed,
        counted,
        staged,
        table_has_region,
    })))
}

/// `recorded` is strictly newer than `incoming` (unparsable versions never
/// block a run).
fn is_newer(recorded: &str, incoming: &str) -> bool {
    compare_version(recorded, incoming).unwrap_or(false)
}

async fn target_task(
    p: Prepared,
    region: ServerRegion,
    manifest: Arc<MasterManifest>,
    started_at: chrono::DateTime<chrono::Utc>,
    mut rx: mpsc::Receiver<Msg>,
) -> TaskEnd {
    let target = p.target.clone();
    let upper = region.as_str().to_uppercase();
    let result = run_target(&p, region, &manifest, started_at, &mut rx).await;
    match result {
        Ok(Some(outcome)) => {
            if let TargetOutcome::Ingested {
                written,
                removed,
                staged,
                ..
            } = &outcome
            {
                info!(
                    "{} Target {} ingested contentHash {} (dataVersion {}): {} file(s) written, \
                     {} removed{}",
                    upper,
                    target.name,
                    manifest.content_hash,
                    manifest.data_version,
                    written,
                    removed,
                    if *staged { ", staged" } else { "" }
                );
            }
            TaskEnd::Outcome(outcome)
        }
        Ok(None) => TaskEnd::Abandoned,
        Err(e) => {
            let error = format!("contentHash {}: {e:#}", manifest.content_hash);
            warn!("{} Target {} failed: {}", upper, target.name, error);
            if let Err(e) = target.record_failure(region, &error).await {
                warn!(
                    "{} Target {}: recording the failure failed: {}",
                    upper, target.name, e
                );
            }
            TaskEnd::Outcome(TargetOutcome::Failed { error })
        }
    }
}

/// The writer side of one target: `Ok(None)` when the run was abandoned.
async fn run_target(
    p: &Prepared,
    region: ServerRegion,
    manifest: &MasterManifest,
    started_at: chrono::DateTime<chrono::Utc>,
    rx: &mut mpsc::Receiver<Msg>,
) -> Result<Option<TargetOutcome>> {
    let target = &p.target;
    let content_hash = manifest.content_hash.as_str();
    let jobs: HashMap<&str, &WriteJob> =
        p.writes.iter().map(|w| (w.file.name.as_str(), w)).collect();
    let allowed = target
        .allowed_shrink(&target.db, region, content_hash)
        .await?;
    let mut shrink_overrides: Vec<String> = Vec::new();
    let mut unknown: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut steady: Option<DatabaseTransaction> = None;
    if p.staged {
        target.mark_staging(region).await?;
    } else {
        steady = Some(target.db.begin().await?);
    }
    let mut written = 0usize;
    loop {
        let Some(msg) = rx.recv().await else {
            bail!("the run ended before every file was sent");
        };
        match msg {
            Msg::Abandon => return Ok(None),
            Msg::Fail(e) => bail!("{e}"),
            Msg::File {
                name,
                mut batches,
                parsed,
            } => {
                let job = jobs
                    .get(name.as_str())
                    .ok_or_else(|| anyhow!("unexpected file {name}"))?;
                let file_txn = match &steady {
                    Some(_) => None,
                    None => Some(target.db.begin().await?),
                };
                let conn: &DatabaseTransaction = match (&steady, &file_txn) {
                    (Some(t), _) | (None, Some(t)) => t,
                    (None, None) => unreachable!(),
                };
                let write = write_file(
                    conn,
                    &job.plan,
                    region,
                    target.cfg.raw,
                    &mut batches,
                    parsed,
                )
                .await
                .map_err(|e| e.context(format!("writing {name}")))?;
                if let Some(prev) = job.prev_rows.filter(|p| *p > 0) {
                    if (write.rows as f64) < target.cfg.min_ratio * prev as f64 {
                        if allowed.contains(&job.plan.table) {
                            shrink_overrides.push(format!(
                                "{} {} -> {} rows (allow-shrink)",
                                job.plan.table, prev, write.rows
                            ));
                        } else {
                            bail!(
                                "{name}: {} rows, fewer than min_ratio {} of the previous {prev} \
                                 (allow it with allow-shrink for contentHash {content_hash})",
                                write.rows,
                                target.cfg.min_ratio
                            );
                        }
                    }
                }
                if !write.unknown_keys.is_empty() {
                    unknown
                        .entry(job.plan.table.clone())
                        .or_default()
                        .extend(write.unknown_keys);
                }
                target
                    .upsert_state(
                        conn,
                        region,
                        &name,
                        &job.file.sha256,
                        Some(&job.plan.table),
                        &job.plan.mapping_hash,
                        Some(write.rows),
                        None,
                    )
                    .await?;
                if let Some(txn) = file_txn {
                    txn.commit().await?;
                }
                written += 1;
            }
            Msg::Finish => break,
        }
    }
    if written != p.writes.len() {
        bail!(
            "{} of {} changed file(s) were delivered",
            written,
            p.writes.len()
        );
    }
    let txn = match steady {
        Some(t) => t,
        None => target.db.begin().await?,
    };
    let removed = finish(p, &txn, region, manifest).await?;
    for (table, keys) in &unknown {
        warn!(
            "{} Target {}: {} has key(s) with no column: {:?}",
            region.as_str().to_uppercase(),
            target.name,
            table,
            keys
        );
    }
    let note = (!shrink_overrides.is_empty())
        .then(|| format!("allow-shrink applied: {}", shrink_overrides.join("; ")));
    target
        .write_version(
            &txn,
            region,
            content_hash,
            &manifest.data_version,
            manifest.git_commit.as_deref(),
            started_at,
            note.as_deref(),
            &unknown,
        )
        .await?;
    txn.commit().await?;
    Ok(Some(TargetOutcome::Ingested {
        content_hash: content_hash.to_string(),
        data_version: manifest.data_version.clone(),
        written,
        removed,
        staged: p.staged,
    }))
}

/// Skipped-file state, removed files, integrity checks and the
/// never-backwards check, inside the region's final transaction. Returns the
/// number of tables cleared.
async fn finish(
    p: &Prepared,
    txn: &DatabaseTransaction,
    region: ServerRegion,
    manifest: &MasterManifest,
) -> Result<usize> {
    let target = &p.target;
    let content_hash = manifest.content_hash.as_str();
    // Never backwards: the recorded version may have moved while we worked
    // (another process between our lock and a restore, an operator edit).
    if let Some(v) = target.load_version(txn, region, true).await? {
        if let Some(recorded) = v.data_version.as_deref() {
            if is_newer(recorded, &manifest.data_version) {
                bail!(
                    "target moved to dataVersion {recorded}, newer than {}; not going backwards",
                    manifest.data_version
                );
            }
        }
    }
    if p.counted != manifest.files.len() {
        bail!(
            "file count mismatch: manifest lists {}, handled {}",
            manifest.files.len(),
            p.counted
        );
    }
    for (file, table, hash) in &p.skips {
        target
            .upsert_state(
                txn,
                region,
                &file.name,
                &file.sha256,
                table.as_deref(),
                hash,
                None,
                None,
            )
            .await?;
    }
    for name in &p.returned {
        target.set_missing_since(txn, region, name, None).await?;
    }
    let mut cleared = 0usize;
    for (name, row) in &p.removed {
        let Some(table) = row.table_name.as_deref() else {
            target.delete_state(txn, region, name).await?;
            continue;
        };
        if target.is_protected(table) {
            bail!(
                "{name} (table {table}) is missing from the manifest but is protected/required; \
                 its rows are kept and this version is not ingested"
            );
        }
        match row.missing_since.as_deref() {
            Some(since) if since != content_hash => {
                let has_region = p.table_has_region.get(table).copied().unwrap_or(true);
                target.clear_table(txn, table, region, has_region).await?;
                target.delete_state(txn, region, name).await?;
                cleared += 1;
                info!(
                    "{} Target {}: {} missing from two consecutive versions; cleared {}",
                    region.as_str().to_uppercase(),
                    target.name,
                    name,
                    table
                );
            }
            Some(_) => {}
            None => {
                target
                    .set_missing_since(txn, region, name, Some(content_hash))
                    .await?;
            }
        }
    }
    for table in target.required() {
        let Some(has_region) = p.table_has_region.get(table) else {
            bail!("required table {table} does not exist in the target database");
        };
        if !target.has_rows(txn, table, region, *has_region).await? {
            bail!("required table {table} has no rows for the region");
        }
    }
    Ok(cleared)
}

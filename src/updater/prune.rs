//! Removal of master files that upstream stopped shipping.
//!
//! The producer ([`super::master::MasterUpdater`]) and the syncer
//! ([`super::sync::MasterSyncer`]) only ever overwrite `<master_dir>/*.json`,
//! so a table dropped upstream would otherwise stay in the directory — and in
//! the git mirror, the bundle and the registry manifest — forever. After a
//! COMPLETE dump (every split/bundle decoded without error) they call
//! [`prune_stale_master_files`] with the set of files that dump produced;
//! stale `*.json` files are deleted before the git commit and the registry
//! publish, so both record the removal.
//!
//! A deletion is committed and pushed and the version file moves on, so no
//! later run would bring a wrongly deleted table back. Hence the layers:
//! a built-in plus configured protect list, a minimum produced ratio, an
//! absolute cap per run, and (producer) a table must be missing from two
//! consecutive complete dumps. The syncer mirrors its owner, which already
//! applied the two-dump rule, but still honours the protect list and guards.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::config::{ServerConfig, ServerRegion};
use crate::error::AppError;

/// Tables consumers cannot work without; never pruned whatever the dump says.
/// deck-service's required keys (`deck-service/src/registry.rs`), which include
/// the event tracker's `events` and `worldBlooms`, plus Haruki-Cloud's card and
/// gacha lookups.
pub const BUILTIN_PROTECTED_TABLES: &[&str] = &[
    "areaItemLevels",
    "areaItems",
    "areas",
    "cardEpisodes",
    "cards",
    "cardRarities",
    "characterRanks",
    "eventCards",
    "eventDeckBonuses",
    "eventExchangeSummaries",
    "events",
    "eventItems",
    "eventRarityBonusRates",
    "gameCharacters",
    "gameCharacterUnits",
    "honors",
    "masterLessons",
    "musicDifficulties",
    "musics",
    "musicVocals",
    "shopItems",
    "skills",
    "worldBloomDifferentAttributeBonuses",
    "worldBlooms",
    "worldBloomSupportDeckBonuses",
    "cardSupplies",
    "gachas",
];

/// How stale files are pruned for one region.
#[derive(Debug, Clone, PartialEq)]
pub struct PrunePolicy {
    /// `servers.<region>.prune_stale`; false keeps every file (old behaviour).
    pub enabled: bool,
    /// `prune_min_ratio`: refuse when the dump produced fewer than this
    /// fraction of the `*.json` files present.
    pub min_ratio: f64,
    /// `prune_max_files`: refuse when one run would delete more files.
    pub max_files: usize,
    /// Extra protected table names (`prune_protect`), without `.json`.
    pub protect: Vec<String>,
    /// Where the tables missing from the previous complete dump are kept.
    /// `Some` enables the two-consecutive-dumps rule (producer); `None`
    /// deletes on the first miss (syncer, which follows a pruning owner).
    pub pending_path: Option<PathBuf>,
}

impl PrunePolicy {
    /// The syncer's policy: guards and protect list, no two-dump rule.
    pub fn from_config(config: &ServerConfig) -> Self {
        let min_ratio = if (0.0..=1.0).contains(&config.prune_min_ratio) {
            config.prune_min_ratio
        } else {
            warn!(
                "prune_min_ratio {} is outside 0.0-1.0; using {}",
                config.prune_min_ratio,
                crate::config::DEFAULT_PRUNE_MIN_RATIO
            );
            crate::config::DEFAULT_PRUNE_MIN_RATIO
        };
        Self {
            enabled: config.prune_stale,
            min_ratio,
            max_files: config.prune_max_files,
            protect: config
                .prune_protect
                .iter()
                .map(|name| name.trim().trim_end_matches(".json").to_string())
                .filter(|name| !name.is_empty())
                .collect(),
            pending_path: None,
        }
    }

    /// The producer's policy: `from_config` plus the two-dump rule.
    pub fn for_producer(config: &ServerConfig, region: ServerRegion) -> Self {
        let pending = if config.prune_pending_path.trim().is_empty() {
            PathBuf::from("./Data/prune").join(format!("{}.json", region.as_str()))
        } else {
            PathBuf::from(config.prune_pending_path.trim())
        };
        Self {
            pending_path: Some(pending),
            ..Self::from_config(config)
        }
    }

    fn is_protected(&self, file_name: &str) -> bool {
        let stem = file_name.strip_suffix(".json").unwrap_or(file_name);
        BUILTIN_PROTECTED_TABLES.contains(&stem) || self.protect.iter().any(|p| p == stem)
    }
}

impl Default for PrunePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            min_ratio: crate::config::DEFAULT_PRUNE_MIN_RATIO,
            max_files: crate::config::DEFAULT_PRUNE_MAX_FILES,
            protect: Vec::new(),
            pending_path: None,
        }
    }
}

/// What [`prune_stale_master_files`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneOutcome {
    /// `prune_stale: false`.
    Disabled,
    /// The directory also holds the version file, so it is not a pure master
    /// directory and nothing in it is deleted.
    MixedDirectory,
    /// The ratio guard fired (dump too small); nothing was deleted and the
    /// pending set was left alone.
    Refused { produced: usize, existing: usize },
    /// More files were due than `prune_max_files`; nothing was deleted.
    OverCap { due: usize, max: usize },
    /// `deleted` were removed; `pending` are missing for the first time and
    /// are deleted if the next complete dump misses them too.
    Pruned {
        deleted: Vec<String>,
        pending: Vec<String>,
    },
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PendingFile {
    missing: BTreeSet<String>,
}

/// Delete stale `*.json` regular files directly in `master_dir` (those not in
/// `produced`, bare file names such as `cards.json`), subject to `policy`.
/// Never touches dot-files (in-flight temp files), non-JSON files,
/// directories or symlinks. Blocking; call it from the blocking pool.
pub fn prune_stale_master_files(
    master_dir: &Path,
    version_path: &str,
    produced: &HashSet<String>,
    policy: &PrunePolicy,
    region_upper: &str,
) -> Result<PruneOutcome, AppError> {
    if !policy.enabled {
        return Ok(PruneOutcome::Disabled);
    }
    if shares_directory(master_dir, version_path) {
        warn!(
            "{} Not pruning stale master files: version file {} lives in master_dir {}",
            region_upper,
            version_path,
            master_dir.display()
        );
        return Ok(PruneOutcome::MixedDirectory);
    }

    let mut existing = 0usize;
    let mut stale: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(master_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name.starts_with('.') {
            continue;
        }
        // `file_type` does not follow symlinks: only plain files are pruned.
        if !entry.file_type()?.is_file() {
            continue;
        }
        existing += 1;
        if !produced.contains(&name) {
            stale.push(name);
        }
    }
    stale.sort();

    // Count only produced names that are pruning candidates themselves, so
    // names the scan ignores cannot inflate the ratio.
    let kept = existing - stale.len();
    let ratio_ok = (kept as f64) >= policy.min_ratio * existing as f64;
    if kept == 0 || !ratio_ok {
        warn!(
            "{} Refusing to prune stale master files: dump produced {} of the {} files present \
(prune_min_ratio {}); {} would have been deleted",
            region_upper,
            kept,
            existing,
            policy.min_ratio,
            stale.len()
        );
        return Ok(PruneOutcome::Refused {
            produced: kept,
            existing,
        });
    }

    let (protected, candidates): (Vec<String>, Vec<String>) = stale
        .into_iter()
        .partition(|name| policy.is_protected(name));
    if !protected.is_empty() {
        warn!(
            "{} Protected master files missing from the dump, kept: {}",
            region_upper,
            protected.join(", ")
        );
    }

    let (due, pending): (Vec<String>, Vec<String>) = match &policy.pending_path {
        Some(path) => {
            let previous = load_pending(path, region_upper);
            candidates
                .into_iter()
                .partition(|name| previous.missing.contains(name))
        }
        None => (candidates, Vec::new()),
    };

    if due.len() > policy.max_files {
        warn!(
            "{} Refusing to prune stale master files: {} are due, more than prune_max_files {}; \
kept: {}",
            region_upper,
            due.len(),
            policy.max_files,
            due.join(", ")
        );
        // Everything due stays pending so a raised cap acts on the next dump.
        if let Some(path) = &policy.pending_path {
            store_pending(path, due.iter().chain(pending.iter()))?;
        }
        return Ok(PruneOutcome::OverCap {
            due: due.len(),
            max: policy.max_files,
        });
    }

    let mut deleted = Vec::with_capacity(due.len());
    for name in due {
        match std::fs::remove_file(master_dir.join(&name)) {
            Ok(()) => {
                info!("{} Pruned stale master file {}", region_upper, name);
                deleted.push(name);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if !deleted.is_empty() {
        info!(
            "{} Pruned {} stale master file(s) missing from consecutive complete dumps",
            region_upper,
            deleted.len()
        );
    }
    if let Some(path) = &policy.pending_path {
        if !pending.is_empty() {
            info!(
                "{} Master files missing from this dump, deleted if the next one misses them \
too: {}",
                region_upper,
                pending.join(", ")
            );
        }
        store_pending(path, pending.iter())?;
    }
    Ok(PruneOutcome::Pruned { deleted, pending })
}

/// The pending set, empty when absent or unreadable (which only delays a
/// deletion by one more dump).
fn load_pending(path: &Path, region_upper: &str) -> PendingFile {
    match std::fs::read(path) {
        Ok(data) => serde_json::from_slice(&data).unwrap_or_else(|e| {
            warn!(
                "{} Ignoring unreadable prune pending file {}: {}",
                region_upper,
                path.display(),
                e
            );
            PendingFile::default()
        }),
        Err(_) => PendingFile::default(),
    }
}

fn store_pending<'a>(path: &Path, names: impl Iterator<Item = &'a String>) -> Result<(), AppError> {
    let file = PendingFile {
        missing: names.cloned().collect(),
    };
    if file.missing.is_empty() && !path.exists() {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_vec_pretty(&file)
        .map_err(|e| AppError::ParseError(format!("prune pending: {e}")))?;
    let tmp = dir.join(format!(".prune-pending.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, json)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn shares_directory(master_dir: &Path, version_path: &str) -> bool {
    if version_path.is_empty() {
        return false;
    }
    let Some(parent) = Path::new(version_path).parent() else {
        return false;
    };
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    match (parent.canonicalize(), master_dir.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("haruki_prune_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn names(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn fill(dir: &Path, list: &[&str]) {
        for name in list {
            std::fs::write(dir.join(name), "[]").unwrap();
        }
    }

    fn pruned(deleted: &[&str], pending: &[&str]) -> PruneOutcome {
        PruneOutcome::Pruned {
            deleted: deleted.iter().map(|s| s.to_string()).collect(),
            pending: pending.iter().map(|s| s.to_string()).collect(),
        }
    }

    const ABCD: [&str; 4] = ["a.json", "b.json", "c.json", "d.json"];

    #[test]
    fn prunes_only_unproduced_plain_json_files() {
        let root = temp_dir();
        let master = root.join("master");
        std::fs::create_dir_all(master.join("sub.json")).unwrap();
        fill(&master, &ABCD);
        fill(&master, &["old.json"]);
        std::fs::write(master.join("README.md"), "x").unwrap();
        std::fs::write(master.join(".tmp.json"), "x").unwrap();
        let version = root.join("version.json");
        let outcome = prune_stale_master_files(
            &master,
            version.to_str().unwrap(),
            &names(&ABCD),
            &PrunePolicy::default(),
            "T",
        )
        .unwrap();
        assert_eq!(outcome, pruned(&["old.json"], &[]));
        assert!(!master.join("old.json").exists());
        assert!(master.join("a.json").exists());
        assert!(master.join("README.md").exists());
        assert!(master.join(".tmp.json").exists());
        assert!(master.join("sub.json").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletes_only_after_two_consecutive_complete_dumps() {
        let root = temp_dir();
        let master = root.join("master");
        std::fs::create_dir_all(&master).unwrap();
        fill(&master, &ABCD);
        fill(&master, &["gone.json", "flaky.json"]);
        let policy = PrunePolicy {
            min_ratio: 0.5,
            pending_path: Some(root.join("state/pending.json")),
            ..PrunePolicy::default()
        };
        let dump = names(&ABCD);
        // First miss: remembered, nothing deleted.
        assert_eq!(
            prune_stale_master_files(&master, "", &dump, &policy, "T").unwrap(),
            pruned(&[], &["flaky.json", "gone.json"])
        );
        assert!(master.join("gone.json").exists());
        // flaky.json comes back in the next dump; gone.json is missed again.
        let mut next = dump.clone();
        next.insert("flaky.json".to_string());
        assert_eq!(
            prune_stale_master_files(&master, "", &next, &policy, "T").unwrap(),
            pruned(&["gone.json"], &[])
        );
        assert!(!master.join("gone.json").exists());
        assert!(master.join("flaky.json").exists());
        // flaky.json missing once more starts over: pending, not deleted.
        assert_eq!(
            prune_stale_master_files(&master, "", &dump, &policy, "T").unwrap(),
            pruned(&[], &["flaky.json"])
        );
        assert!(master.join("flaky.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn protected_tables_are_never_pruned() {
        let root = temp_dir();
        fill(&root, &ABCD);
        fill(&root, &["x.json", "y.json", "z.json", "w.json"]);
        fill(
            &root,
            &["events.json", "worldBlooms.json", "custom.json", "old.json"],
        );
        let policy = PrunePolicy {
            min_ratio: 0.5,
            protect: vec!["custom".to_string()],
            ..PrunePolicy::default()
        };
        let mut dump = names(&ABCD);
        dump.extend(names(&["x.json", "y.json", "z.json", "w.json"]));
        assert_eq!(
            prune_stale_master_files(&root, "", &dump, &policy, "T").unwrap(),
            pruned(&["old.json"], &[])
        );
        for name in ["events.json", "worldBlooms.json", "custom.json"] {
            assert!(root.join(name).exists(), "{name} protected");
        }
        let mut config: ServerConfig = serde_yaml::from_str("{}").unwrap();
        config.prune_protect = vec![" extra.json ".to_string(), String::new()];
        assert_eq!(PrunePolicy::from_config(&config).protect, ["extra"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cap_refuses_the_whole_prune_and_keeps_files_pending() {
        let root = temp_dir();
        let master = root.join("master");
        std::fs::create_dir_all(&master).unwrap();
        let produced: Vec<String> = (0..40).map(|i| format!("t{i}.json")).collect();
        for name in &produced {
            std::fs::write(master.join(name), "[]").unwrap();
        }
        fill(&master, &["s1.json", "s2.json", "s3.json"]);
        let dump: HashSet<String> = produced.into_iter().collect();
        let policy = PrunePolicy {
            max_files: 2,
            pending_path: Some(root.join("pending.json")),
            ..PrunePolicy::default()
        };
        assert_eq!(
            prune_stale_master_files(&master, "", &dump, &policy, "T").unwrap(),
            pruned(&[], &["s1.json", "s2.json", "s3.json"])
        );
        assert_eq!(
            prune_stale_master_files(&master, "", &dump, &policy, "T").unwrap(),
            PruneOutcome::OverCap { due: 3, max: 2 }
        );
        assert!(master.join("s1.json").exists());
        // Raising the cap deletes them on the next dump.
        let raised = PrunePolicy {
            max_files: 3,
            ..policy
        };
        assert_eq!(
            prune_stale_master_files(&master, "", &dump, &raised, "T").unwrap(),
            pruned(&["s1.json", "s2.json", "s3.json"], &[])
        );
        // Without the two-dump rule the cap applies on the first miss.
        fill(&master, &["s1.json", "s2.json", "s3.json"]);
        let syncer = PrunePolicy {
            max_files: 2,
            ..PrunePolicy::default()
        };
        assert_eq!(
            prune_stale_master_files(&master, "", &dump, &syncer, "T").unwrap(),
            PruneOutcome::OverCap { due: 3, max: 2 }
        );
        assert!(master.join("s3.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ratio_guard_refuses_mass_deletion_and_empty_dumps() {
        let root = temp_dir();
        fill(&root, &ABCD);
        let loose = PrunePolicy {
            min_ratio: 0.5,
            ..PrunePolicy::default()
        };
        assert_eq!(
            prune_stale_master_files(
                &root,
                "",
                &names(&["a.json", "b.json"]),
                &PrunePolicy::default(),
                "T"
            )
            .unwrap(),
            PruneOutcome::Refused {
                produced: 2,
                existing: 4
            }
        );
        let empty = prune_stale_master_files(&root, "", &HashSet::new(), &loose, "T").unwrap();
        assert!(matches!(empty, PruneOutcome::Refused { produced: 0, .. }));
        // Names the scan ignores cannot inflate the ratio.
        let inflated = names(&["a.json", ".x.json", ".y.json", "z.txt", "missing.json"]);
        assert_eq!(
            prune_stale_master_files(&root, "", &inflated, &PrunePolicy::default(), "T").unwrap(),
            PruneOutcome::Refused {
                produced: 1,
                existing: 4
            }
        );
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 4);
        assert_eq!(
            prune_stale_master_files(&root, "", &names(&["a.json", "b.json"]), &loose, "T")
                .unwrap(),
            pruned(&["c.json", "d.json"], &[])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_values_are_validated() {
        let mut config: ServerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(PrunePolicy::from_config(&config), PrunePolicy::default());
        for bad in [f64::NAN, -0.1, 1.5, f64::INFINITY] {
            config.prune_min_ratio = bad;
            assert_eq!(
                PrunePolicy::from_config(&config).min_ratio,
                crate::config::DEFAULT_PRUNE_MIN_RATIO
            );
        }
        assert_eq!(
            PrunePolicy::for_producer(&config, ServerRegion::Tw).pending_path,
            Some(PathBuf::from("./Data/prune/tw.json"))
        );
        config.prune_pending_path = "/state/tw.json".to_string();
        assert_eq!(
            PrunePolicy::for_producer(&config, ServerRegion::Tw).pending_path,
            Some(PathBuf::from("/state/tw.json"))
        );
    }

    #[test]
    fn disabled_or_mixed_directory_keeps_everything() {
        let root = temp_dir();
        fill(&root, &["a.json", "old.json"]);
        let disabled = PrunePolicy {
            enabled: false,
            min_ratio: 0.0,
            ..PrunePolicy::default()
        };
        assert_eq!(
            prune_stale_master_files(&root, "", &names(&["a.json"]), &disabled, "T").unwrap(),
            PruneOutcome::Disabled
        );
        let version = root.join("version.json");
        let loose = PrunePolicy {
            min_ratio: 0.0,
            ..PrunePolicy::default()
        };
        assert_eq!(
            prune_stale_master_files(
                &root,
                version.to_str().unwrap(),
                &names(&["a.json"]),
                &loose,
                "T"
            )
            .unwrap(),
            PruneOutcome::MixedDirectory
        );
        assert!(root.join("old.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

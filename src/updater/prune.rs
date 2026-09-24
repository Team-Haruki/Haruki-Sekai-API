//! Removal of master files that upstream stopped shipping.
//!
//! The producer ([`super::master::MasterUpdater`]) and the syncer
//! ([`super::sync::MasterSyncer`]) only ever overwrite `<master_dir>/*.json`,
//! so a table dropped upstream would otherwise stay in the directory — and in
//! the git mirror, the bundle and the registry manifest — forever. After a
//! COMPLETE dump (every split/bundle decoded without error) they call
//! [`prune_stale_master_files`] with the set of files that dump produced;
//! anything else matching `*.json` is deleted before the git commit and the
//! registry publish, so both record the removal.

use std::collections::HashSet;
use std::path::Path;

use tracing::{info, warn};

use crate::config::ServerConfig;
use crate::error::AppError;

/// How stale files are pruned for one region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrunePolicy {
    /// `servers.<region>.prune_stale`; false keeps every file (old behaviour).
    pub enabled: bool,
    /// `servers.<region>.prune_min_ratio`: refuse to prune when the dump
    /// produced fewer than this fraction of the `*.json` files present, so a
    /// truncated-but-"successful" dump can never wipe the directory.
    pub min_ratio: f64,
}

impl PrunePolicy {
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
        }
    }
}

impl Default for PrunePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            min_ratio: crate::config::DEFAULT_PRUNE_MIN_RATIO,
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
    /// The mass-deletion guard fired; nothing was deleted.
    Refused { produced: usize, existing: usize },
    /// These file names were deleted (possibly none).
    Pruned(Vec<String>),
}

/// Delete every `*.json` regular file directly in `master_dir` that is not in
/// `produced` (bare file names such as `cards.json`). Never touches
/// dot-files (in-flight temp files), non-JSON files, directories or symlinks.
/// Blocking; call it from the blocking pool.
pub fn prune_stale_master_files(
    master_dir: &Path,
    version_path: &str,
    produced: &HashSet<String>,
    policy: PrunePolicy,
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

    stale.sort();
    let mut pruned = Vec::with_capacity(stale.len());
    for name in stale {
        match std::fs::remove_file(master_dir.join(&name)) {
            Ok(()) => {
                info!("{} Pruned stale master file {}", region_upper, name);
                pruned.push(name);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if !pruned.is_empty() {
        info!(
            "{} Pruned {} stale master file(s) not produced by the latest dump",
            region_upper,
            pruned.len()
        );
    }
    Ok(PruneOutcome::Pruned(pruned))
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

    #[test]
    fn prunes_only_unproduced_plain_json_files() {
        let root = temp_dir();
        let master = root.join("master");
        std::fs::create_dir_all(master.join("sub.json")).unwrap();
        for name in ["a.json", "b.json", "c.json", "d.json", "old.json"] {
            std::fs::write(master.join(name), "[]").unwrap();
        }
        std::fs::write(master.join("README.md"), "x").unwrap();
        std::fs::write(master.join(".tmp.json"), "x").unwrap();
        let version = root.join("version.json");
        let outcome = prune_stale_master_files(
            &master,
            version.to_str().unwrap(),
            &names(&["a.json", "b.json", "c.json", "d.json"]),
            PrunePolicy::default(),
            "T",
        )
        .unwrap();
        assert_eq!(outcome, PruneOutcome::Pruned(vec!["old.json".to_string()]));
        assert!(!master.join("old.json").exists());
        assert!(master.join("a.json").exists());
        assert!(master.join("README.md").exists());
        assert!(master.join(".tmp.json").exists());
        assert!(master.join("sub.json").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn guard_refuses_mass_deletion_and_empty_dumps() {
        let root = temp_dir();
        for name in ["a.json", "b.json", "c.json", "d.json"] {
            std::fs::write(root.join(name), "[]").unwrap();
        }
        let refused = prune_stale_master_files(
            &root,
            "",
            &names(&["a.json", "b.json"]),
            PrunePolicy::default(),
            "T",
        )
        .unwrap();
        assert_eq!(
            refused,
            PruneOutcome::Refused {
                produced: 2,
                existing: 4
            }
        );
        let empty =
            prune_stale_master_files(&root, "", &HashSet::new(), PrunePolicy::default(), "T")
                .unwrap();
        assert!(matches!(empty, PruneOutcome::Refused { produced: 0, .. }));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 4);

        // A lower ratio lets the same dump through.
        let loose = PrunePolicy {
            enabled: true,
            min_ratio: 0.5,
        };
        let pruned =
            prune_stale_master_files(&root, "", &names(&["a.json", "b.json"]), loose, "T").unwrap();
        assert_eq!(
            pruned,
            PruneOutcome::Pruned(vec!["c.json".to_string(), "d.json".to_string()])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignored_names_do_not_inflate_the_ratio_and_bad_ratios_fall_back() {
        let root = temp_dir();
        for name in ["a.json", "b.json", "c.json", "d.json"] {
            std::fs::write(root.join(name), "[]").unwrap();
        }
        let produced = names(&["a.json", ".x.json", ".y.json", "z.txt", "missing.json"]);
        assert_eq!(
            prune_stale_master_files(&root, "", &produced, PrunePolicy::default(), "T").unwrap(),
            PruneOutcome::Refused {
                produced: 1,
                existing: 4
            }
        );
        let mut config: ServerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(PrunePolicy::from_config(&config), PrunePolicy::default());
        for bad in [f64::NAN, -0.1, 1.5, f64::INFINITY] {
            config.prune_min_ratio = bad;
            assert_eq!(
                PrunePolicy::from_config(&config).min_ratio,
                crate::config::DEFAULT_PRUNE_MIN_RATIO
            );
        }
        config.prune_min_ratio = 0.5;
        config.prune_stale = false;
        assert_eq!(
            PrunePolicy::from_config(&config),
            PrunePolicy {
                enabled: false,
                min_ratio: 0.5
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disabled_or_mixed_directory_keeps_everything() {
        let root = temp_dir();
        std::fs::write(root.join("a.json"), "[]").unwrap();
        std::fs::write(root.join("old.json"), "[]").unwrap();
        let disabled = PrunePolicy {
            enabled: false,
            min_ratio: 0.0,
        };
        assert_eq!(
            prune_stale_master_files(&root, "", &names(&["a.json"]), disabled, "T").unwrap(),
            PruneOutcome::Disabled
        );
        let version = root.join("version.json");
        let loose = PrunePolicy {
            enabled: true,
            min_ratio: 0.0,
        };
        assert_eq!(
            prune_stale_master_files(
                &root,
                version.to_str().unwrap(),
                &names(&["a.json"]),
                loose,
                "T"
            )
            .unwrap(),
            PruneOutcome::MixedDirectory
        );
        assert!(root.join("old.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

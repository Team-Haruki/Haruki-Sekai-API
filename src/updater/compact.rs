//! Lossless, row-streamed download variants of the two largest JP catalogs.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use serde_json::{Map, Value};

use crate::error::AppError;

const EXPORTS: [(&str, &str, &str); 2] = [
    ("cards", "compactCards", "cardParameters"),
    ("gachas", "compactGachas", "gachaDetails"),
];

/// Backfill missing exports and rebuild those older than their source. The
/// syncer preserves source mtimes on byte-identical updates. On a new bundle,
/// only export sources actually present in that bundle, and include the
/// derived names in its produced set before pruning.
pub(super) fn ensure_exports(
    master_dir: &Path,
    sources: Option<&HashSet<String>>,
) -> Result<(bool, HashSet<String>), AppError> {
    let mut changed = false;
    let mut produced = HashSet::new();
    for (source, target, field) in EXPORTS {
        let source_name = format!("{source}.json");
        if sources.is_some_and(|names| !names.contains(&source_name)) {
            continue;
        }
        let source_path = master_dir.join(source_name);
        let source_meta = match std::fs::metadata(&source_path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let target_name = format!("{target}.json");
        let target_path = master_dir.join(&target_name);
        let fresh = match std::fs::metadata(&target_path) {
            Ok(meta) => meta.len() > 0 && meta.modified()? >= source_meta.modified()?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };
        if !fresh {
            write_export(&source_path, &target_path, field)?;
            changed = true;
        }
        produced.insert(target_name);
    }
    Ok((changed, produced))
}

fn write_export(source: &Path, target: &Path, field: &str) -> Result<(), AppError> {
    let tmp = target.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), AppError> {
        let reader = BufReader::with_capacity(256 * 1024, std::fs::File::open(source)?);
        let mut writer = BufWriter::with_capacity(256 * 1024, std::fs::File::create(&tmp)?);
        writer.write_all(b"[")?;
        let mut first = true;
        crate::ingest_engine::stream_rows(reader, 1, |rows| {
            for mut row in rows {
                if let Some(value) = row.get_mut(field) {
                    if let Some(compact) = compact_columns(value) {
                        *value = compact;
                    }
                }
                if !first {
                    writer.write_all(b",")?;
                }
                first = false;
                serde_json::to_writer(&mut writer, &row)?;
            }
            Ok(())
        })
        .map_err(|e| AppError::ParseError(format!("compact master export: {e}")))?;
        writer.write_all(b"]")?;
        writer.flush()?;
        drop(writer);
        std::fs::rename(&tmp, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Use the Nuverse column + `__ENUM__` convention, retaining every field and
/// value. Heterogeneous rows stay as rows: missing keys must not become nulls.
fn compact_columns(value: &Value) -> Option<Value> {
    let rows = value.as_array().filter(|rows| !rows.is_empty())?;
    let first = rows.first()?.as_object()?;
    if first.is_empty() || first.contains_key("__ENUM__") {
        return None;
    }
    if !rows.iter().all(|row| {
        row.as_object().is_some_and(|row| {
            row.len() == first.len() && first.keys().all(|key| row.contains_key(key))
        })
    }) {
        return None;
    }
    let mut columns = Map::new();
    let mut enums = Map::new();
    for key in first.keys() {
        let values = rows.iter().map(|row| row[key].clone()).collect::<Vec<_>>();
        let mut lookup = HashMap::new();
        let mut dictionary = Vec::new();
        let mut indices = Vec::with_capacity(values.len());
        for value in &values {
            let encoded = serde_json::to_string(value).ok()?;
            let index = *lookup.entry(encoded).or_insert_with(|| {
                dictionary.push(value.clone());
                dictionary.len() - 1
            });
            indices.push(Value::from(index));
        }
        // Include the enum key and envelope overhead; don't enlarge unique
        // columns such as IDs. Dictionary order is first occurrence order.
        let plain_len = serde_json::to_vec(&values).ok()?.len();
        let indexed_len = serde_json::to_vec(&indices).ok()?.len()
            + serde_json::to_vec(&dictionary).ok()?.len()
            + serde_json::to_vec(key).ok()?.len()
            + 16;
        if indexed_len < plain_len {
            columns.insert(key.clone(), Value::Array(indices));
            enums.insert(key.clone(), Value::Array(dictionary));
        } else {
            columns.insert(key.clone(), Value::Array(values));
        }
    }
    // An empty dictionary is also a marker, distinguishing this encoding
    // from a future native object-shaped field in the unmodified source.
    columns.insert("__ENUM__".to_string(), Value::Object(enums));
    let compact = Value::Object(columns);
    (serde_json::to_vec(&compact).ok()?.len() < serde_json::to_vec(value).ok()?.len())
        .then_some(compact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::nuverse_schema::NuverseSchemaStore;
    use serde_json::json;

    fn restore(value: Value) -> Value {
        let store =
            NuverseSchemaStore::from_slice(br#"{"master":{},"api":[],"schemas":[]}"#).unwrap();
        store
            .restore_master_table("compactRows".into(), value)
            .unwrap()
            .pop()
            .unwrap()
            .1
    }

    #[test]
    fn round_trip_columns_and_enums_without_losing_values() {
        let rows = Value::Array(
            (0..80)
                .map(|id| {
                    json!({
                        "id": id, "cardId": 12345, "type": "param1", "power": id * 2,
                        "null": null, "wish": false, "nested": {"x": [null, 2]},
                    })
                })
                .collect(),
        );
        let compact = compact_columns(&rows).unwrap();
        assert!(compact["__ENUM__"]["type"].is_array());
        assert_eq!(restore(compact.clone()), rows);
        assert_eq!(compact_columns(&rows).unwrap(), compact);
    }

    #[test]
    fn preserves_shapes_that_cannot_be_compacted_losslessly() {
        for value in [
            json!([]),
            json!({"param1": [1]}),
            json!([1, 2]),
            json!([{}, {}]),
            json!([{"x": 1}, {"y": 2}]),
            json!([{"x": null}, {}]),
            json!([{"__ENUM__": 1}]),
            json!([{"x": 1}]),
        ] {
            assert!(compact_columns(&value).is_none(), "{value}");
        }
    }

    #[test]
    fn backfill_is_atomic_idempotent_and_tracks_bundle_sources() {
        let dir = std::env::temp_dir().join(format!("haruki_compact_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("cards.json");
        let target = dir.join("compactCards.json");
        let params = (0..60)
            .map(|id| json!({"id": id, "power": id, "cardId": 1}))
            .collect::<Vec<_>>();
        let original = json!([{"id": 1, "cardParameters": params, "extra": [null, {"a": 2}]}]);
        std::fs::write(&source, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
        let (changed, names) = ensure_exports(&dir, None).unwrap();
        assert!(changed);
        assert_eq!(names, HashSet::from(["compactCards.json".into()]));
        let bytes = std::fs::read(&target).unwrap();
        let mut compact: Value = serde_json::from_slice(&bytes).unwrap();
        compact[0]["cardParameters"] = restore(compact[0]["cardParameters"].take());
        assert_eq!(compact, original);
        let modified = std::fs::metadata(&target).unwrap().modified().unwrap();
        assert!(!ensure_exports(&dir, None).unwrap().0);
        assert_eq!(
            std::fs::metadata(&target).unwrap().modified().unwrap(),
            modified
        );
        assert!(ensure_exports(&dir, Some(&HashSet::new()))
            .unwrap()
            .1
            .is_empty());
        std::fs::write(&source, b"[{\"cardParameters\": []}, broken]").unwrap();
        assert!(write_export(&source, &target, "cardParameters").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "requires HARUKI_TEST_COMPACT_MASTER_DIR with real JP cards/gachas"]
    fn real_catalog_round_trip() {
        let dir =
            std::path::PathBuf::from(std::env::var("HARUKI_TEST_COMPACT_MASTER_DIR").unwrap());
        for (source, target, field) in EXPORTS {
            let source = dir.join(format!("{source}.json"));
            let target = dir.join(format!("{target}.json"));
            write_export(&source, &target, field).unwrap();
            let original: Value = serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
            let mut exported: Value =
                serde_json::from_slice(&std::fs::read(target).unwrap()).unwrap();
            for row in exported.as_array_mut().unwrap() {
                if row[field].get("__ENUM__").is_some() {
                    row[field] = restore(row[field].take());
                }
            }
            assert_eq!(exported, original);
        }
    }
}

use anyhow::{Context, Result};
use futures::StreamExt;
use sea_orm::sea_query::{Alias, Expr, ExprTrait, InsertStatement, Query};
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

// Structure of schema_info.json
#[derive(serde::Deserialize)]
struct TableInfo {
    name: String,
    columns: Vec<String>,
    #[serde(default)]
    unique_keys: Option<Vec<Vec<String>>>,
}

type ColumnTypeMap = HashMap<String, String>;
type UniqueKeys = Vec<Vec<String>>;
type SchemaMap = HashMap<String, (ColumnTypeMap, UniqueKeys)>;

pub struct IngestionEngine {
    db: DatabaseConnection,
    schema_map: SchemaMap, // table -> (column -> type, unique_keys)
    file_to_table: HashMap<String, String>,
}

impl IngestionEngine {
    pub async fn new(db: DatabaseConnection) -> Result<Self> {
        let schema_json =
            fs::read_to_string("schema_info.json").context("Failed to read schema_info.json")?;
        let tables: Vec<TableInfo> = serde_json::from_str(&schema_json)?;

        let mut schema_map = HashMap::new();
        let mut file_to_table = HashMap::new();

        for table in tables {
            let mut col_map = HashMap::new();
            for col_type_str in table.columns {
                if let Some((col, typ)) = col_type_str.split_once(':') {
                    col_map.insert(col.to_string(), typ.to_string());
                } else {
                    col_map.insert(col_type_str.clone(), "string".to_string());
                }
            }
            schema_map.insert(
                table.name.clone(),
                (col_map, table.unique_keys.clone().unwrap_or_default()),
            );

            let no_underscores = table.name.replace("_", "");
            file_to_table.insert(no_underscores.clone(), table.name.clone());
            if table.name.ends_with('s') {
                file_to_table.insert(
                    table.name[..table.name.len() - 1].replace("_", ""),
                    table.name.clone(), // Fallback rule handling Go pluralizations
                );
            }
        }

        Ok(Self {
            db,
            schema_map,
            file_to_table,
        })
    }

    fn resolve_table_name(&self, file_name_without_ext: &str) -> Option<String> {
        let normalized = file_name_without_ext.to_lowercase().replace("_", "");

        if let Some(tbl) = self.file_to_table.get(&normalized) {
            return Some(tbl.clone());
        }
        let mut with_s = normalized.clone();
        with_s.push('s');
        if let Some(tbl) = self.file_to_table.get(&with_s) {
            return Some(tbl.clone());
        }
        let mut with_es = normalized.clone();
        with_es.push_str("es");
        if let Some(tbl) = self.file_to_table.get(&with_es) {
            return Some(tbl.clone());
        }
        None
    }

    /// Ingest all JSON files in `dir_path` for the given `region`, running up to
    /// `CONCURRENCY` files concurrently. Each file is processed in its own transaction
    /// (DELETE existing region rows → batch INSERT new rows), so a failure in one file
    /// does not roll back others.
    pub async fn ingest_master_data(&self, dir_path: &str, region: &str) -> Result<()> {
        let path = Path::new(dir_path);
        if !path.exists() || !path.is_dir() {
            warn!("Directory {} does not exist", dir_path);
            return Ok(());
        }

        let mut json_files: Vec<PathBuf> = Vec::new();
        let mut rd = tokio::fs::read_dir(path).await?;
        while let Some(entry) = rd.next_entry().await? {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                json_files.push(p);
            }
        }

        // Process up to CONCURRENCY files at a time. The connection pool will queue
        // transactions that exceed its max_connections; no failures from contention.
        const CONCURRENCY: usize = 8;
        let failed_tables: Vec<String> = futures::stream::iter(json_files)
            .map(|p| async move {
                match self.ingest_file(&p, region).await {
                    Ok(()) => None,
                    Err(e) => {
                        warn!("Failed to ingest {}: {:#}", p.display(), e);
                        Some(p.display().to_string())
                    }
                }
            })
            .buffer_unordered(CONCURRENCY)
            .filter_map(|r| async move { r })
            .collect()
            .await;

        if !failed_tables.is_empty() {
            // Surface the failure to the caller. The master updater treats ingest
            // as best-effort (files on disk and the git mirror track the download,
            // not DB health) but records the failure and retries the ingest on its
            // next cron tick; the CLI reports it per region.
            anyhow::bail!(
                "ingestion failed for {} file(s): {:?}",
                failed_tables.len(),
                failed_tables
            );
        }
        info!("Successfully ingested all master data files for {}", region);
        Ok(())
    }

    async fn ingest_file(&self, path: &Path, region: &str) -> Result<()> {
        let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) else {
            warn!(
                "{} Skipping {}: non-UTF-8 filename",
                region.to_uppercase(),
                path.display()
            );
            return Ok(());
        };
        let table_name = match self.resolve_table_name(file_stem) {
            Some(t) => t,
            None => return Ok(()),
        };

        if matches!(
            table_name.as_str(),
            "character_profiles" | "virtual_items" | "virtualitems"
        ) {
            return Ok(());
        }

        let (db_cols, _unique_keys) = self.schema_map.get(&table_name).unwrap();
        let has_server_region = db_cols.contains_key("server_region");

        // Async file I/O — does not block the runtime.
        let json_content = tokio::fs::read_to_string(path).await?;

        // JSON parsing and row-value building are CPU-bound; run on the blocking thread pool
        // so they don't starve other async tasks running concurrently.
        let db_cols_owned = db_cols.clone();
        let region_str = region.to_string();
        let table_name_for_build = table_name.clone();
        let (column_names, rows) = tokio::task::spawn_blocking(move || {
            build_insert_data(
                &json_content,
                &table_name_for_build,
                &db_cols_owned,
                &region_str,
                has_server_region,
            )
        })
        .await??;

        if rows.is_empty() {
            return Ok(());
        }

        // Everything below is I/O-bound DB work — stays on the async executor.
        let txn = self
            .db
            .begin()
            .await
            .context("Failed to begin transaction")?;

        if has_server_region {
            let mut del = Query::delete();
            del.from_table(Alias::new(&table_name))
                .and_where(Expr::col(Alias::new("server_region")).eq(region));
            txn.execute(&del)
                .await
                .context("Failed to delete existing region data")?;
        } else {
            let mut del = Query::delete();
            del.from_table(Alias::new(&table_name));
            txn.execute(&del).await.context("Failed to clear table")?;
        }

        let mut insert_stmt = InsertStatement::new()
            .into_table(Alias::new(&table_name))
            .to_owned();
        insert_stmt.columns(column_names.iter().map(|n| Alias::new(n.as_str())));

        // PostgreSQL limits bind parameters to 65535 per query.
        // Divide by column count (minimum 1) to stay safely under the limit.
        let batch_size = (65_535 / column_names.len().max(1)).clamp(1, 5_000);
        let mut rows_iter = rows.into_iter();
        loop {
            let chunk: Vec<Vec<sea_orm::sea_query::SimpleExpr>> =
                rows_iter.by_ref().take(batch_size).collect();
            if chunk.is_empty() {
                break;
            }
            let mut batch = insert_stmt.clone();
            for row in chunk {
                batch.values_panic(row);
            }
            txn.execute(&batch)
                .await
                .context("Failed to execute batch insert")?;
        }

        txn.commit().await.context("Failed to commit transaction")?;
        Ok(())
    }
}

/// CPU-bound work extracted for `spawn_blocking`: parse JSON, map keys to DB columns,
/// and build typed row values. Returns (ordered column names, rows of SimpleExpr).
fn build_insert_data(
    json_content: &str,
    table_name: &str,
    db_cols: &HashMap<String, String>,
    region: &str,
    has_server_region: bool,
) -> Result<(Vec<String>, Vec<Vec<sea_orm::sea_query::SimpleExpr>>)> {
    let data: Vec<Value> = serde_json::from_str(json_content)?;
    if data.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let all_json_keys = collect_json_keys(&data);
    let target_columns = map_target_columns(&all_json_keys, db_cols);

    let mut column_names: Vec<String> = target_columns.iter().map(|c| c.db_col.clone()).collect();
    if has_server_region {
        column_names.push("server_region".to_string());
    }

    let rows = build_rows(
        &data,
        table_name,
        &target_columns,
        region,
        has_server_region,
        column_names.len(),
    );

    Ok((column_names, rows))
}

struct MappedCol {
    json_key: String,
    db_col: String,
    col_type: String,
}

fn collect_json_keys(data: &[Value]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    for obj in data.iter().filter_map(Value::as_object) {
        for key in obj.keys() {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

fn map_target_columns(keys: &[String], db_cols: &HashMap<String, String>) -> Vec<MappedCol> {
    keys.iter()
        .filter_map(|json_key| {
            let normalized = match normalize_json_key(json_key).as_str() {
                "id" => "gameid".to_string(),
                other => other.to_string(),
            };
            let db_col = db_cols
                .keys()
                .find(|column| normalize_db_col(column) == normalized)?;
            Some(MappedCol {
                json_key: json_key.clone(),
                db_col: db_col.clone(),
                col_type: db_cols[db_col].clone(),
            })
        })
        .collect()
}

fn build_rows(
    data: &[Value],
    table_name: &str,
    target_columns: &[MappedCol],
    region: &str,
    has_server_region: bool,
    row_capacity: usize,
) -> Vec<Vec<sea_orm::sea_query::SimpleExpr>> {
    data.iter()
        .filter_map(Value::as_object)
        .map(|obj| {
            let mut row = Vec::with_capacity(row_capacity);
            for column in target_columns {
                let value = obj.get(&column.json_key).unwrap_or(&Value::Null);
                row.push(
                    json_to_sea_value_for_column(
                        table_name,
                        obj,
                        &column.db_col,
                        value,
                        &column.col_type,
                    )
                    .into(),
                );
            }
            if has_server_region {
                let region_value: sea_orm::sea_query::Value = region.into();
                row.push(region_value.into());
            }
            row
        })
        .collect()
}

fn normalize_json_key(key: &str) -> String {
    key.trim_start_matches('_').to_lowercase().replace("_", "")
}

fn normalize_db_col(col: &str) -> String {
    col.to_lowercase().replace("_", "")
}

fn json_to_sea_value_for_column(
    table_name: &str,
    obj: &serde_json::Map<String, Value>,
    db_col: &str,
    val: &Value,
    col_type: &str,
) -> sea_orm::sea_query::Value {
    if table_name == "cards" && db_col == "assetbundle_name" {
        if let Some(assetbundle_name) = preferred_card_assetbundle_name(obj, val) {
            return assetbundle_name.into();
        }
    }

    json_to_sea_value(val, col_type)
}

fn preferred_card_assetbundle_name(
    obj: &serde_json::Map<String, Value>,
    fallback: &Value,
) -> Option<String> {
    if let Some(archive_display_type) = obj.get("archiveDisplayType").and_then(Value::as_str) {
        if is_card_resource_name(archive_display_type) {
            return Some(archive_display_type.to_string());
        }
    }

    fallback
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
}

fn is_card_resource_name(value: &str) -> bool {
    let value = value.trim();
    let Some(rest) = value.strip_prefix("res") else {
        return false;
    };
    let Some((character_part, card_part)) = rest.split_once("_no") else {
        return false;
    };

    !character_part.is_empty()
        && !card_part.is_empty()
        && character_part.bytes().all(|b| b.is_ascii_digit())
        && card_part.bytes().all(|b| b.is_ascii_digit())
}

fn json_to_sea_value(val: &Value, col_type: &str) -> sea_orm::sea_query::Value {
    if col_type == "json.RawMessage" {
        return if val.is_null() {
            sea_orm::sea_query::Value::Json(None)
        } else {
            sea_orm::sea_query::Value::Json(Some(Box::new(val.clone())))
        };
    }
    if val.is_null() {
        return null_value_for_col_type(col_type);
    }

    match col_type {
        "int64" | "int32" | "int" => integer_value(val),
        "float64" | "float32" | "float" => float_value(val),
        "bool" => bool_value(val),
        "string" => string_value(val),
        _ => inferred_value(val),
    }
}

fn integer_value(val: &Value) -> sea_orm::sea_query::Value {
    match val {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok()))
            .map(Into::into)
            .unwrap_or_else(|| sea_orm::sea_query::Value::BigInt(None)),
        Value::String(s) => s
            .trim()
            .parse::<i64>()
            .ok()
            .map(Into::into)
            .unwrap_or_else(|| sea_orm::sea_query::Value::BigInt(None)),
        _ => sea_orm::sea_query::Value::BigInt(None),
    }
}

fn float_value(val: &Value) -> sea_orm::sea_query::Value {
    match val {
        Value::Number(n) => n
            .as_f64()
            .map(Into::into)
            .unwrap_or_else(|| sea_orm::sea_query::Value::Double(None)),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .map(Into::into)
            .unwrap_or_else(|| sea_orm::sea_query::Value::Double(None)),
        _ => sea_orm::sea_query::Value::Double(None),
    }
}

fn bool_value(val: &Value) -> sea_orm::sea_query::Value {
    match val {
        Value::Bool(b) => (*b).into(),
        Value::String(s) => s
            .trim()
            .parse::<bool>()
            .ok()
            .map(Into::into)
            .unwrap_or_else(|| sea_orm::sea_query::Value::Bool(None)),
        _ => sea_orm::sea_query::Value::Bool(None),
    }
}

fn string_value(val: &Value) -> sea_orm::sea_query::Value {
    match val {
        Value::String(s) => s.as_str().into(),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(val).unwrap_or_default().into()
        }
        Value::Null => sea_orm::sea_query::Value::String(None),
    }
}

fn inferred_value(val: &Value) -> sea_orm::sea_query::Value {
    match val {
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => n
            .as_i64()
            .map(Into::into)
            .or_else(|| n.as_f64().map(Into::into))
            .unwrap_or_else(|| n.to_string().into()),
        Value::String(s) => s.as_str().into(),
        Value::Array(_) | Value::Object(_) => {
            sea_orm::sea_query::Value::Json(Some(Box::new(val.clone())))
        }
        Value::Null => sea_orm::sea_query::Value::Json(None),
    }
}

fn null_value_for_col_type(col_type: &str) -> sea_orm::sea_query::Value {
    match col_type {
        "int64" | "int32" | "int" => sea_orm::sea_query::Value::BigInt(None),
        "float64" | "float32" | "float" => sea_orm::sea_query::Value::Double(None),
        "bool" => sea_orm::sea_query::Value::Bool(None),
        "string" => sea_orm::sea_query::Value::String(None),
        _ => sea_orm::sea_query::Value::Json(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn prefers_archive_display_type_for_card_assetbundle_name() {
        let obj = json!({
            "assetbundleName": "localized title",
            "archiveDisplayType": "res017_no037"
        })
        .as_object()
        .unwrap()
        .clone();

        let value = json_to_sea_value_for_column(
            "cards",
            &obj,
            "assetbundle_name",
            obj.get("assetbundleName").unwrap(),
            "string",
        );

        match value {
            sea_orm::sea_query::Value::String(Some(s)) => assert_eq!(&*s, "res017_no037"),
            other => panic!("unexpected value: {other:?}"),
        }
    }

    #[test]
    fn invalid_scalar_values_become_null_for_typed_columns() {
        let int_from_text = json_to_sea_value(&json!("not an integer"), "int64");
        let int_from_object = json_to_sea_value(&json!({"param1": [1, 2, 3]}), "int64");

        assert!(matches!(
            int_from_text,
            sea_orm::sea_query::Value::BigInt(None)
        ));
        assert!(matches!(
            int_from_object,
            sea_orm::sea_query::Value::BigInt(None)
        ));
    }

    #[tokio::test]
    #[ignore] // Requires a running local Postgres; run with: cargo test -- --ignored
    async fn test_direct_ingestion() -> anyhow::Result<()> {
        let mut opt =
            ConnectOptions::new("postgres://haruki:sekai@localhost:5432/master_data".to_owned());
        opt.max_connections(5)
            .min_connections(1)
            .connect_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(8));

        let db = Database::connect(opt).await?;
        let engine = IngestionEngine::new(db).await?;

        println!("Ingesting jp region data...");
        engine.ingest_master_data("master_data/jp", "jp").await?;
        Ok(())
    }

    #[test]
    fn test_normalize_json_key_trims_leading_underscore() {
        assert_eq!(normalize_json_key("_assetbundleName"), "assetbundlename");
        assert_eq!(normalize_json_key("assetbundleName"), "assetbundlename");
        assert_eq!(normalize_db_col("assetbundle_name"), "assetbundlename");
    }

    #[test]
    fn builds_insert_columns_rows_and_region() {
        let mut columns = HashMap::new();
        columns.insert("game_id".to_string(), "int64".to_string());
        columns.insert("display_name".to_string(), "string".to_string());
        columns.insert("enabled".to_string(), "bool".to_string());
        columns.insert("server_region".to_string(), "string".to_string());

        let (names, rows) = build_insert_data(
            r#"[{"id":1,"displayName":"A","enabled":true},{"id":"2","displayName":"B"},null]"#,
            "items",
            &columns,
            "jp",
            true,
        )
        .unwrap();

        assert!(names.contains(&"game_id".to_string()));
        assert!(names.contains(&"display_name".to_string()));
        assert!(names.contains(&"enabled".to_string()));
        assert_eq!(names.last().unwrap(), "server_region");
        assert_eq!(rows.len(), 2);
        assert!(build_insert_data("[]", "items", &columns, "jp", true)
            .unwrap()
            .0
            .is_empty());
        assert!(build_insert_data("invalid", "items", &columns, "jp", true).is_err());
    }

    #[test]
    fn collects_unique_keys_and_maps_known_columns() {
        let data = vec![
            json!({"id": 1, "displayName": "A"}),
            json!({"id": 2, "extra": 3}),
        ];
        assert_eq!(collect_json_keys(&data), vec!["id", "displayName", "extra"]);

        let mut columns = HashMap::new();
        columns.insert("game_id".to_string(), "int64".to_string());
        columns.insert("display_name".to_string(), "string".to_string());
        let mapped = map_target_columns(&collect_json_keys(&data), &columns);
        assert_eq!(mapped.len(), 2);
        assert!(mapped.iter().any(|column| column.db_col == "game_id"));
        assert!(mapped.iter().any(|column| column.db_col == "display_name"));
    }

    #[test]
    fn card_resource_name_validation_and_fallbacks() {
        assert!(is_card_resource_name("res017_no037"));
        for invalid in ["", "res_no1", "res1_no", "foo1_no2", "resx_no2", "res1_nox"] {
            assert!(!is_card_resource_name(invalid), "{invalid}");
        }

        let valid = json!({"archiveDisplayType": "res001_no002"});
        assert_eq!(
            preferred_card_assetbundle_name(valid.as_object().unwrap(), &Value::Null).as_deref(),
            Some("res001_no002")
        );
        let fallback = json!({"archiveDisplayType": "archive"});
        assert_eq!(
            preferred_card_assetbundle_name(fallback.as_object().unwrap(), &json!("bundle"))
                .as_deref(),
            Some("bundle")
        );
        assert_eq!(
            preferred_card_assetbundle_name(fallback.as_object().unwrap(), &json!("   ")),
            None
        );
    }

    #[test]
    fn converts_every_supported_column_value_shape() {
        use sea_orm::sea_query::Value as SeaValue;

        assert!(matches!(
            json_to_sea_value(&Value::Null, "int32"),
            SeaValue::BigInt(None)
        ));
        assert!(matches!(
            json_to_sea_value(&Value::Null, "float"),
            SeaValue::Double(None)
        ));
        assert!(matches!(
            json_to_sea_value(&Value::Null, "bool"),
            SeaValue::Bool(None)
        ));
        assert!(matches!(
            json_to_sea_value(&Value::Null, "string"),
            SeaValue::String(None)
        ));
        assert!(matches!(
            json_to_sea_value(&Value::Null, "unknown"),
            SeaValue::Json(None)
        ));
        assert!(matches!(
            json_to_sea_value(&Value::Null, "json.RawMessage"),
            SeaValue::Json(None)
        ));
        assert!(matches!(
            json_to_sea_value(&json!({"x": 1}), "json.RawMessage"),
            SeaValue::Json(Some(_))
        ));

        assert!(matches!(
            integer_value(&json!(12)),
            SeaValue::BigInt(Some(12))
        ));
        assert!(matches!(
            integer_value(&json!(" 13 ")),
            SeaValue::BigInt(Some(13))
        ));
        assert!(matches!(
            integer_value(&json!(true)),
            SeaValue::BigInt(None)
        ));
        assert!(matches!(float_value(&json!(1.5)), SeaValue::Double(Some(v)) if v == 1.5));
        assert!(matches!(float_value(&json!(" 2.5 ")), SeaValue::Double(Some(v)) if v == 2.5));
        assert!(matches!(float_value(&json!([])), SeaValue::Double(None)));
        assert!(matches!(
            bool_value(&json!(true)),
            SeaValue::Bool(Some(true))
        ));
        assert!(matches!(
            bool_value(&json!(" false ")),
            SeaValue::Bool(Some(false))
        ));
        assert!(matches!(bool_value(&json!(0)), SeaValue::Bool(None)));

        assert!(matches!(string_value(&json!("text")), SeaValue::String(Some(v)) if &*v == "text"));
        assert!(matches!(
            string_value(&json!([1, 2])),
            SeaValue::String(Some(_))
        ));
        assert!(matches!(
            inferred_value(&json!(true)),
            SeaValue::Bool(Some(true))
        ));
        assert!(matches!(
            inferred_value(&json!(7)),
            SeaValue::BigInt(Some(7))
        ));
        assert!(matches!(inferred_value(&json!(1.25)), SeaValue::Double(Some(v)) if v == 1.25));
        assert!(matches!(
            inferred_value(&json!("s")),
            SeaValue::String(Some(_))
        ));
        assert!(matches!(
            inferred_value(&json!({"x": 1})),
            SeaValue::Json(Some(_))
        ));
    }

    #[tokio::test]
    async fn loads_schema_resolves_tables_and_skips_non_ingestable_files() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let engine = IngestionEngine::new(db).await.unwrap();
        assert!(engine.resolve_table_name("cards").is_some());
        assert!(engine.resolve_table_name("card").is_some());
        assert!(engine.resolve_table_name("definitely_unknown").is_none());
        assert!(engine
            .ingest_master_data("/definitely/missing/master", "jp")
            .await
            .is_ok());

        let root = std::env::temp_dir().join(format!("haruki_ingest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("unknown.json"), "[]").unwrap();
        std::fs::write(root.join("ignored.txt"), "[]").unwrap();
        assert!(engine
            .ingest_master_data(root.to_str().unwrap(), "jp")
            .await
            .is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}

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

pub struct IngestionEngine {
    db: DatabaseConnection,
    schema_map: HashMap<String, (HashMap<String, String>, Vec<Vec<String>>)>, // table -> (column -> type, unique_keys)
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
            warn!("Completed with failures in: {:?}", failed_tables);
        } else {
            info!("Successfully ingested all master data files for {}", region);
        }

        Ok(())
    }

    async fn ingest_file(&self, path: &Path, region: &str) -> Result<()> {
        let file_stem = path.file_stem().unwrap().to_str().unwrap();
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
        let (column_names, rows) = tokio::task::spawn_blocking(move || {
            build_insert_data(&json_content, &db_cols_owned, &region_str, has_server_region)
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
            txn.execute(&del)
                .await
                .context("Failed to clear table")?;
        }

        let mut insert_stmt = InsertStatement::new()
            .into_table(Alias::new(&table_name))
            .to_owned();
        insert_stmt.columns(column_names.iter().map(|n| Alias::new(n.as_str())));

        // PostgreSQL limits bind parameters to 65535 per query.
        // Divide by column count (minimum 1) to stay safely under the limit.
        let batch_size = (65_535 / column_names.len().max(1)).min(5_000).max(1);
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
    db_cols: &HashMap<String, String>,
    region: &str,
    has_server_region: bool,
) -> Result<(Vec<String>, Vec<Vec<sea_orm::sea_query::SimpleExpr>>)> {
    let data: Vec<Value> = serde_json::from_str(json_content)?;
    if data.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // Collect ALL unique JSON keys across all records (not just the first one),
    // because some fields only appear in a subset of records.
    let mut all_json_keys: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for item in &data {
        if let Value::Object(obj) = item {
            for key in obj.keys() {
                if seen.insert(key.clone()) {
                    all_json_keys.push(key.clone());
                }
            }
        }
    }

    struct MappedCol {
        json_key: String,
        db_col: String,
        col_type: String,
    }

    let mut target_columns: Vec<MappedCol> = Vec::new();
    for json_key in &all_json_keys {
        let mut norm = json_key.to_lowercase().replace("_", "");
        if norm == "id" {
            norm = "gameid".to_string();
        }
        if let Some(db_col) = db_cols.keys().find(|k| k.replace("_", "") == norm) {
            target_columns.push(MappedCol {
                json_key: json_key.clone(),
                col_type: db_cols[db_col].clone(),
                db_col: db_col.clone(),
            });
        }
        // Skip JSON keys that don't match any known DB column
    }

    let mut column_names: Vec<String> = target_columns.iter().map(|c| c.db_col.clone()).collect();
    if has_server_region {
        column_names.push("server_region".to_string());
    }

    let mut rows: Vec<Vec<sea_orm::sea_query::SimpleExpr>> = Vec::with_capacity(data.len());
    for item in &data {
        if let Value::Object(obj) = item {
            let mut row: Vec<sea_orm::sea_query::SimpleExpr> =
                Vec::with_capacity(column_names.len());
            for mcol in &target_columns {
                let val = obj.get(&mcol.json_key).unwrap_or(&Value::Null);
                row.push(json_to_sea_value(val, &mcol.col_type).into());
            }
            if has_server_region {
                let region_val: sea_orm::sea_query::Value = region.into();
                row.push(region_val.into());
            }
            rows.push(row);
        }
    }

    Ok((column_names, rows))
}

fn json_to_sea_value(val: &Value, col_type: &str) -> sea_orm::sea_query::Value {
    if col_type == "json.RawMessage" {
        return sea_orm::sea_query::Value::Json(Some(Box::new(val.clone())));
    }
    match val {
        Value::Null => match col_type {
            "int64" | "int32" | "int" => sea_orm::sea_query::Value::BigInt(None),
            "float64" | "float32" | "float" => sea_orm::sea_query::Value::Double(None),
            "bool" => sea_orm::sea_query::Value::Bool(None),
            "string" => sea_orm::sea_query::Value::String(None),
            _ => sea_orm::sea_query::Value::Json(None),
        },
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(f) = n.as_f64() {
                f.into()
            } else {
                n.to_string().into()
            }
        }
        Value::String(s) => s.as_str().into(),
        Value::Array(arr) => {
            let json_str = serde_json::to_string(arr).unwrap_or_default();
            sea_orm::sea_query::Value::Json(Some(Box::new(
                serde_json::from_str(&json_str).unwrap(),
            )))
        }
        Value::Object(obj) => {
            let json_str = serde_json::to_string(obj).unwrap_or_default();
            sea_orm::sea_query::Value::Json(Some(Box::new(
                serde_json::from_str(&json_str).unwrap(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};
    use std::time::Duration;

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
}

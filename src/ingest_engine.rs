use anyhow::{Context, Result};
use sea_orm::sea_query::{Alias, InsertStatement};
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
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

    fn to_sea_query_value(val: &Value, col_type: &str) -> sea_orm::sea_query::Value {
        if col_type == "json.RawMessage" {
            // Strict enforce JSONB cast
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

    pub async fn ingest_master_data(&self, dir_path: &str, region: &str) -> Result<()> {
        let path = Path::new(dir_path);
        if !path.exists() || !path.is_dir() {
            warn!("Directory {} does not exist", dir_path);
            return Ok(());
        }

        let mut entries = fs::read_dir(path)?;
        let mut failed_tables = Vec::new();

        while let Some(entry) = entries.next() {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Err(e) = self.ingest_file(&path, region).await {
                    warn!("Failed to ingest {}: {:#}", path.display(), e);
                    failed_tables.push(path.display().to_string());
                }
            }
        }

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
            None => {
                // Not mapped, skip silently
                return Ok(());
            }
        };

        if table_name == "character_profiles"
            || table_name == "virtual_items"
            || table_name == "virtualitems"
        {
            return Ok(());
        }

        let (db_cols, _unique_keys) = self.schema_map.get(&table_name).unwrap();

        let json_content = fs::read_to_string(path)?;
        let data: Vec<Value> = serde_json::from_str(&json_content)?;

        if data.is_empty() {
            return Ok(());
        }

        let mut insert_stmt = InsertStatement::new()
            .into_table(Alias::new(&table_name))
            .to_owned();

        struct MappedCol {
            json_key: String,
            db_col: String,
            col_type: String,
        }

        let mut target_columns: Vec<MappedCol> = Vec::new();
        let first_obj = data[0].as_object().unwrap();

        for json_key in first_obj.keys() {
            let mut normalized_json_key = json_key.to_lowercase().replace("_", "");
            if normalized_json_key == "id" {
                normalized_json_key = "gameid".to_string();
            }

            let mut actual_db_col = None;
            for db_c in db_cols.keys() {
                if db_c.replace("_", "") == normalized_json_key {
                    actual_db_col = Some(db_c.clone());
                    break;
                }
            }

            if let Some(db_col) = actual_db_col {
                target_columns.push(MappedCol {
                    json_key: json_key.clone(),
                    col_type: db_cols.get(&db_col).unwrap().clone(),
                    db_col,
                });
            } else {
                let snake_key = {
                    let mut s = String::new();
                    for (i, c) in json_key.chars().enumerate() {
                        if c.is_uppercase() && i > 0 {
                            s.push('_');
                        }
                        s.push(c.to_ascii_lowercase());
                    }
                    s
                };

                if snake_key != "id" {
                    target_columns.push(MappedCol {
                        json_key: json_key.clone(),
                        db_col: snake_key,
                        col_type: "json.RawMessage".to_string(), // Fallback
                    });
                }
            }
        }

        let mut column_aliases = Vec::new();
        for mcol in &target_columns {
            column_aliases.push(Alias::new(&mcol.db_col));
        }

        let has_server_region = db_cols.contains_key("server_region");
        if has_server_region {
            column_aliases.push(Alias::new("server_region"));
        }

        insert_stmt.columns(column_aliases);

        let mut rows_to_insert = Vec::new();

        for item in &data {
            if let Value::Object(obj) = item {
                let mut row = Vec::new();
                for mcol in &target_columns {
                    let val = obj.get(&mcol.json_key).unwrap_or(&Value::Null);
                    row.push(Self::to_sea_query_value(val, &mcol.col_type).into());
                }
                if has_server_region {
                    row.push(region.into());
                }

                rows_to_insert.push(row);
            }
        }

        // Use a transaction: DELETE existing rows for this region, then INSERT fresh snapshot.
        // Master data is always a complete snapshot per region, so a full replace is correct.
        let txn = self
            .db
            .begin()
            .await
            .context("Failed to begin transaction")?;

        // Delete existing rows for this table+region before inserting
        if has_server_region {
            let delete_sql = format!(
                "DELETE FROM \"{}\" WHERE server_region = '{}'",
                table_name, region
            );
            txn.execute_unprepared(&delete_sql)
                .await
                .context("Failed to delete existing region data")?;
        } else {
            // No server_region column — truncate all rows (rare edge case)
            let delete_sql = format!("DELETE FROM \"{}\"", table_name);
            txn.execute_unprepared(&delete_sql)
                .await
                .context("Failed to truncate table")?;
        }

        // Batch insert in chunks of 1000
        let mut current_chunk = Vec::new();

        for row in rows_to_insert {
            current_chunk.push(row);

            if current_chunk.len() >= 1000 {
                let mut chunk_stmt = insert_stmt.clone();
                for r in current_chunk.drain(..) {
                    chunk_stmt.values_panic(r);
                }

                txn.execute(&chunk_stmt)
                    .await
                    .context("Failed to execute batch insert")?;
            }
        }

        if !current_chunk.is_empty() {
            let mut chunk_stmt = insert_stmt.clone();
            for r in current_chunk.drain(..) {
                chunk_stmt.values_panic(r);
            }

            txn.execute(&chunk_stmt)
                .await
                .context("Failed to execute batch insert")?;
        }

        txn.commit().await.context("Failed to commit transaction")?;

        Ok(())
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

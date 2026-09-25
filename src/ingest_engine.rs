use anyhow::{Context, Result};
use futures::StreamExt;
use sea_orm::sea_query::{Alias, Expr, ExprTrait, InsertStatement, Query};
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

// Structure of schema_info.json
#[derive(serde::Deserialize, serde::Serialize)]
struct TableInfo {
    name: String,
    columns: Vec<String>,
    #[serde(default)]
    unique_keys: Option<Vec<Vec<String>>>,
}

pub(crate) type ColumnTypeMap = HashMap<String, String>;
pub(crate) type UniqueKeys = Vec<Vec<String>>;
type SchemaMap = HashMap<String, (ColumnTypeMap, UniqueKeys)>;

/// Files ingested concurrently by default. Each in-flight file holds at most
/// a few row batches in memory, so this bounds the ingest footprint on a
/// shared node; raise it via `master_database.ingest_concurrency` where RAM
/// allows.
pub const DEFAULT_INGEST_CONCURRENCY: usize = 2;
/// Rows parsed per batch before they are turned into an INSERT.
pub(crate) const ROWS_PER_BATCH: usize = 2_000;
/// Estimated in-memory size (see [`approx_value_bytes`]) at which a batch is
/// cut before it reaches `ROWS_PER_BATCH`. Row counts alone do not bound
/// memory: `gachas.json` or `cards.json` are 35-48 MB in ~1-1.5k rows, so a
/// row-count batch was the whole file as a `Value` tree (several times its size),
/// copied again into typed values and the INSERT. With this cap, memory per
/// in-flight file is about `(CHANNEL_DEPTH + 2) * BATCH_BYTES` plus the typed
/// copy of one batch, whatever the file size.
pub(crate) const BATCH_BYTES: usize = 1024 * 1024;
/// Parsed batches allowed to queue ahead of the inserting side.
pub(crate) const CHANNEL_DEPTH: usize = 2;

/// Tables dropped from the schema long ago whose files are never ingested.
/// Note the generator would name a future characterProfiles model
/// `characterprofiles` (not in this list) but a virtualItems model
/// `virtualitems` (in it) — remove the entry before adding that model.
pub(crate) fn is_legacy_skipped_table(table: &str) -> bool {
    matches!(
        table,
        "character_profiles" | "virtual_items" | "virtualitems"
    )
}

/// The typed table map from `schema_info.json`: table -> (column -> type,
/// unique keys), plus the file-stem resolution rules.
pub struct MasterSchema {
    schema_map: SchemaMap,
    file_to_table: HashMap<String, String>,
    /// Canonical JSON of each table's schema entry (mapping fingerprints).
    entries: HashMap<String, String>,
}

impl MasterSchema {
    pub async fn load(path: &str) -> Result<Self> {
        let schema_json = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read {path}"))?;
        Self::parse(&schema_json)
    }

    pub fn parse(schema_json: &str) -> Result<Self> {
        let tables: Vec<TableInfo> = serde_json::from_str(schema_json)?;

        let mut schema_map = HashMap::new();
        let mut file_to_table = HashMap::new();
        let mut entries = HashMap::new();

        for table in tables {
            let mut sorted_columns = table.columns.clone();
            sorted_columns.sort();
            entries.insert(
                table.name.clone(),
                serde_json::json!({
                    "columns": sorted_columns,
                    "unique_keys": table.unique_keys,
                })
                .to_string(),
            );
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
            schema_map,
            file_to_table,
            entries,
        })
    }

    pub fn resolve_table_name(&self, file_name_without_ext: &str) -> Option<String> {
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
        // The generator pluralizes a singular `…y` alias to `…ies`
        // (`streamingLiveCategory` -> `streaminglivecategories`).
        if let Some(stem) = normalized.strip_suffix('y') {
            if let Some(tbl) = self.file_to_table.get(&format!("{stem}ies")) {
                return Some(tbl.clone());
            }
        }
        None
    }

    /// Column -> type map and unique keys of `table`.
    pub fn table(&self, table: &str) -> Option<&(ColumnTypeMap, UniqueKeys)> {
        self.schema_map.get(table)
    }

    /// Canonical JSON of `table`'s schema entry (columns sorted).
    pub fn entry_fingerprint(&self, table: &str) -> Option<&str> {
        self.entries.get(table).map(String::as_str)
    }

    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.schema_map.keys().map(String::as_str)
    }
}

pub struct IngestionEngine {
    db: DatabaseConnection,
    schema: MasterSchema,
    concurrency: usize,
}

impl IngestionEngine {
    pub async fn new(db: DatabaseConnection) -> Result<Self> {
        Ok(Self {
            db,
            schema: MasterSchema::load("schema_info.json").await?,
            concurrency: DEFAULT_INGEST_CONCURRENCY,
        })
    }

    /// Number of files ingested at once (minimum 1).
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    fn resolve_table_name(&self, file_name_without_ext: &str) -> Option<String> {
        self.schema.resolve_table_name(file_name_without_ext)
    }

    /// Ingest all JSON files in `dir_path` for the given `region`, running up to
    /// `concurrency` files at once. Each file is processed in its own transaction
    /// (DELETE existing region rows → batch INSERT new rows), so a failure in one file
    /// does not roll back others. Rows are parsed and inserted in bounded batches,
    /// so a 50 MB table never has to be materialized whole.
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

        // The connection pool queues transactions that exceed its
        // max_connections; no failures from contention.
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
            .buffer_unordered(self.concurrency)
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

        // Legacy hard skip kept on purpose (see `is_legacy_skipped_table`).
        if is_legacy_skipped_table(&table_name) {
            return Ok(());
        }

        let (db_cols, _unique_keys) = self.schema.table(&table_name).unwrap();
        let has_server_region = db_cols.contains_key("server_region");

        // Parse on the blocking pool, streaming the JSON array in row batches
        // through a bounded channel; the async side turns each batch into
        // INSERTs inside one transaction. Peak memory per file is a couple of
        // batches, independent of the table size.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Batch>(CHANNEL_DEPTH);
        let file = tokio::fs::File::open(path).await?.into_std().await;
        let db_cols_owned = db_cols.clone();
        let region_owned = region.to_string();
        let table_for_parse = table_name.clone();
        let parser = tokio::task::spawn_blocking(move || -> Result<usize> {
            let reader = std::io::BufReader::with_capacity(256 * 1024, file);
            let mut total = 0usize;
            stream_rows(reader, ROWS_PER_BATCH, |rows| {
                total += rows.len();
                let batch = build_batch(
                    &rows,
                    &table_for_parse,
                    &db_cols_owned,
                    &region_owned,
                    has_server_region,
                );
                if batch.rows.is_empty() {
                    return Ok(());
                }
                tx.blocking_send(batch)
                    .map_err(|_| anyhow::anyhow!("ingest receiver closed"))
            })?;
            Ok(total)
        });

        let result = self
            .insert_batches(&table_name, region, has_server_region, &mut rx)
            .await;
        // Drain so a parser blocked on a full channel can finish and report.
        drop(rx);
        let parsed = parser
            .await
            .map_err(|e| anyhow::anyhow!("ingest parse task: {e}"))?;
        // The transaction is committed only once the parser reports success:
        // a file that fails to parse after some batches were inserted rolls
        // back instead of replacing the region with a truncated row set.
        match (result, parsed) {
            (Err(e), _) => Err(e),
            (Ok(txn), Err(e)) => {
                drop(txn);
                Err(e.context(format!("parsing {}", path.display())))
            }
            (Ok(Some(txn)), Ok(_)) => txn.commit().await.context("Failed to commit transaction"),
            (Ok(None), Ok(_)) => Ok(()),
        }
    }

    /// Receive parsed batches and write them under one transaction, which is
    /// returned uncommitted (`None` when no batch arrived: an empty file
    /// leaves the table untouched, matching the previous whole-file
    /// behavior). The region DELETE only runs once the first batch arrives.
    async fn insert_batches(
        &self,
        table_name: &str,
        region: &str,
        has_server_region: bool,
        rx: &mut tokio::sync::mpsc::Receiver<Batch>,
    ) -> Result<Option<sea_orm::DatabaseTransaction>> {
        let Some(first) = rx.recv().await else {
            return Ok(None);
        };
        let txn = self
            .db
            .begin()
            .await
            .context("Failed to begin transaction")?;

        if has_server_region {
            let mut del = Query::delete();
            del.from_table(Alias::new(table_name))
                .and_where(Expr::col(Alias::new("server_region")).eq(region));
            txn.execute(&del)
                .await
                .context("Failed to delete existing region data")?;
        } else {
            let mut del = Query::delete();
            del.from_table(Alias::new(table_name));
            txn.execute(&del).await.context("Failed to clear table")?;
        }

        let mut next = Some(first);
        while let Some(batch) = next.take() {
            insert_batch(&txn, table_name, batch).await?;
            next = rx.recv().await;
        }
        Ok(Some(txn))
    }
}

/// One parsed row batch: the INSERT column list (derived from the keys
/// present in this batch) and the typed row values.
pub(crate) struct Batch {
    pub(crate) column_names: Vec<String>,
    pub(crate) rows: Vec<Vec<sea_orm::sea_query::SimpleExpr>>,
}

pub(crate) async fn insert_batch(
    txn: &impl ConnectionTrait,
    table_name: &str,
    batch: Batch,
) -> Result<()> {
    let mut insert_stmt = InsertStatement::new()
        .into_table(Alias::new(table_name))
        .to_owned();
    insert_stmt.columns(batch.column_names.iter().map(|n| Alias::new(n.as_str())));
    // PostgreSQL limits bind parameters to 65535 per query.
    // Divide by column count (minimum 1) to stay safely under the limit.
    let chunk_rows = (65_535 / batch.column_names.len().max(1)).clamp(1, 5_000);
    let mut rows_iter = batch.rows.into_iter();
    loop {
        let chunk: Vec<Vec<sea_orm::sea_query::SimpleExpr>> =
            rows_iter.by_ref().take(chunk_rows).collect();
        if chunk.is_empty() {
            break;
        }
        let mut stmt = insert_stmt.clone();
        for row in chunk {
            stmt.values_panic(row);
        }
        // Build, then drop the statement: its values are cloned into the
        // built statement, so holding both doubles the batch in memory.
        let built = txn.get_database_backend().build(&stmt);
        drop(stmt);
        txn.execute_raw(built)
            .await
            .context("Failed to execute batch insert")?;
    }
    Ok(())
}

/// Stream a JSON array from `reader`, handing `f` the elements in batches of
/// at most `batch_size` rows and about [`BATCH_BYTES`] of parsed values
/// (a single larger row is a batch of its own). Non-object elements are
/// passed through and filtered by the batch builder, as before. The last
/// (partial) batch is flushed at the end of the array.
pub(crate) fn stream_rows<R: std::io::Read>(
    reader: R,
    batch_size: usize,
    f: impl FnMut(Vec<Value>) -> Result<()>,
) -> Result<()> {
    stream_rows_bounded(reader, batch_size, BATCH_BYTES, f)
}

pub(crate) fn stream_rows_bounded<R: std::io::Read>(
    reader: R,
    batch_size: usize,
    batch_bytes: usize,
    f: impl FnMut(Vec<Value>) -> Result<()>,
) -> Result<()> {
    struct RowsVisitor<F> {
        batch_size: usize,
        batch_bytes: usize,
        f: F,
    }
    impl<'de, F: FnMut(Vec<Value>) -> Result<()>> serde::de::Visitor<'de> for RowsVisitor<F> {
        type Value = ();
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a JSON array of master rows")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            mut self,
            mut seq: A,
        ) -> std::result::Result<(), A::Error> {
            let mut buf = Vec::new();
            let mut bytes = 0usize;
            while let Some(row) = seq.next_element::<Value>()? {
                bytes += approx_value_bytes(&row);
                buf.push(row);
                if buf.len() >= self.batch_size || bytes >= self.batch_bytes {
                    (self.f)(std::mem::take(&mut buf)).map_err(serde::de::Error::custom)?;
                    bytes = 0;
                }
            }
            if !buf.is_empty() {
                (self.f)(buf).map_err(serde::de::Error::custom)?;
            }
            Ok(())
        }
    }
    let mut de = serde_json::Deserializer::from_reader(reader);
    serde::Deserializer::deserialize_seq(
        &mut de,
        RowsVisitor {
            batch_size,
            batch_bytes,
            f,
        },
    )?;
    de.end()?;
    Ok(())
}

/// Rough heap footprint of a parsed value (node sizes plus string and key
/// bytes, ignoring allocator slack): what batches are bounded by.
pub(crate) fn approx_value_bytes(value: &Value) -> usize {
    const NODE: usize = std::mem::size_of::<Value>();
    match value {
        Value::String(s) => NODE + s.len(),
        Value::Array(items) => NODE + items.iter().map(approx_value_bytes).sum::<usize>(),
        // Entry: key String, value, hash and index slot.
        Value::Object(map) => {
            NODE + map
                .iter()
                .map(|(k, v)| 40 + k.len() + approx_value_bytes(v))
                .sum::<usize>()
        }
        _ => NODE,
    }
}

/// CPU-bound work for one batch: map the keys present in these rows to DB
/// columns and build typed row values. Columns absent from every row of a
/// batch are omitted from that batch's INSERT (they take the column default,
/// NULL for every master table), which is how missing keys were handled
/// before batching as well.
pub(crate) fn build_batch(
    data: &[Value],
    table_name: &str,
    db_cols: &HashMap<String, String>,
    region: &str,
    has_server_region: bool,
) -> Batch {
    if data.is_empty() {
        return Batch {
            column_names: Vec::new(),
            rows: Vec::new(),
        };
    }

    let all_json_keys = collect_json_keys(data);
    let target_columns = map_target_columns(&all_json_keys, db_cols);

    let mut column_names: Vec<String> = target_columns.iter().map(|c| c.db_col.clone()).collect();
    if has_server_region {
        column_names.push("server_region".to_string());
    }

    let rows = build_rows(
        data,
        table_name,
        &target_columns,
        region,
        has_server_region,
        column_names.len(),
    );

    Batch { column_names, rows }
}

pub(crate) struct MappedCol {
    pub(crate) json_key: String,
    pub(crate) db_col: String,
    pub(crate) col_type: String,
}

pub(crate) fn collect_json_keys(data: &[Value]) -> Vec<String> {
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

pub(crate) fn map_target_columns(
    keys: &[String],
    db_cols: &HashMap<String, String>,
) -> Vec<MappedCol> {
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

pub(crate) fn build_rows(
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

        let data: Vec<Value> = serde_json::from_str(
            r#"[{"id":1,"displayName":"A","enabled":true},{"id":"2","displayName":"B"},null]"#,
        )
        .unwrap();
        let batch = build_batch(&data, "items", &columns, "jp", true);

        assert!(batch.column_names.contains(&"game_id".to_string()));
        assert!(batch.column_names.contains(&"display_name".to_string()));
        assert!(batch.column_names.contains(&"enabled".to_string()));
        assert_eq!(batch.column_names.last().unwrap(), "server_region");
        assert_eq!(batch.rows.len(), 2);
        let empty = build_batch(&[], "items", &columns, "jp", true);
        assert!(empty.column_names.is_empty() && empty.rows.is_empty());
    }

    #[test]
    fn large_rows_cut_batches_by_size_not_only_by_count() {
        // 300 rows of ~20 KB each: one row-count batch would be the whole
        // file; the size cap splits it and keeps every row, in order.
        let big = "x".repeat(20_000);
        let rows: Vec<Value> = (0..300).map(|i| json!({"id": i, "blob": big})).collect();
        let json = serde_json::to_vec(&rows).unwrap();
        let row_bytes = approx_value_bytes(&rows[0]);
        assert!(row_bytes > 20_000);
        let cap = 100 * row_bytes;
        let mut sizes = Vec::new();
        let mut seen = 0;
        stream_rows_bounded(std::io::Cursor::new(json), ROWS_PER_BATCH, cap, |batch| {
            let bytes: usize = batch.iter().map(approx_value_bytes).sum();
            assert!(bytes <= cap, "batch of {bytes} bytes over the {cap} cap");
            for row in &batch {
                assert_eq!(row["id"], json!(seen));
                seen += 1;
            }
            sizes.push(batch.len());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 300);
        assert_eq!(sizes, vec![100, 100, 100]);

        // A row larger than the cap is a batch of its own.
        let mut sizes = Vec::new();
        let json = serde_json::to_vec(&rows[..3]).unwrap();
        stream_rows_bounded(std::io::Cursor::new(json), ROWS_PER_BATCH, 1, |batch| {
            sizes.push(batch.len());
            Ok(())
        })
        .unwrap();
        assert_eq!(sizes, vec![1, 1, 1]);
    }

    #[test]
    fn streams_rows_in_bounded_batches_and_rejects_bad_json() {
        let json = r#"[{"id":1},{"id":2},{"id":3},null,{"id":5}]"#;
        let mut batches = Vec::new();
        stream_rows(std::io::Cursor::new(json), 2, |rows| {
            batches.push(rows);
            Ok(())
        })
        .unwrap();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 2);
        assert_eq!(batches[2], vec![json!({"id": 5})]);

        let mut none = 0;
        stream_rows(std::io::Cursor::new("[]"), 2, |_| {
            none += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(none, 0);

        assert!(stream_rows(std::io::Cursor::new("invalid"), 2, |_| Ok(())).is_err());
        assert!(stream_rows(std::io::Cursor::new(r#"{"a":1}"#), 2, |_| Ok(())).is_err());
        assert!(stream_rows(std::io::Cursor::new("[1,2] trailing"), 2, |_| Ok(())).is_err());
        // A truncated array fails after the complete batches were delivered.
        let mut seen = 0;
        assert!(stream_rows(std::io::Cursor::new("[1,2,3"), 2, |rows| {
            seen += rows.len();
            Ok(())
        })
        .is_err());
        assert_eq!(seen, 2);
        // Callback errors propagate.
        assert!(stream_rows(std::io::Cursor::new("[1]"), 1, |_| {
            anyhow::bail!("stop")
        })
        .is_err());
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

    /// End-to-end against an in-memory SQLite: batches cross the
    /// ROWS_PER_BATCH boundary, per-batch column sets differ, a broken file
    /// rolls back, and an empty file leaves the table alone.
    #[tokio::test]
    async fn ingests_in_batches_with_rollback_and_empty_file_semantics() {
        use sea_orm::Statement;
        let mut opt = ConnectOptions::new("sqlite::memory:".to_string());
        opt.max_connections(1);
        let db = Database::connect(opt).await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE bonds (game_id INTEGER, group_id INTEGER, character_id1 INTEGER, \
             character_id2 INTEGER, server_region TEXT)",
        )
        .await
        .unwrap();
        let engine = IngestionEngine::new(db.clone()).await.unwrap();
        let root = std::env::temp_dir().join(format!("haruki_ingest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let count = |region: &'static str| {
            let db = db.clone();
            async move {
                let row = db
                    .query_one_raw(Statement::from_string(
                        db.get_database_backend(),
                        format!(
                            "SELECT COUNT(*) AS n, SUM(character_id2 IS NULL) AS nulls FROM bonds \
                             WHERE server_region = '{region}'"
                        ),
                    ))
                    .await
                    .unwrap()
                    .unwrap();
                (
                    row.try_get::<i64>("", "n").unwrap(),
                    row.try_get::<i64>("", "nulls").unwrap(),
                )
            }
        };

        // Rows 0..2500 carry characterId2, the rest do not: the second batch's
        // INSERT omits that column and those rows read back NULL.
        let total = ROWS_PER_BATCH * 2 + 500;
        let rows: Vec<Value> = (0..total)
            .map(|i| {
                if i < 2500 {
                    json!({"id": i, "groupId": 1, "characterId1": 1, "characterId2": 2})
                } else {
                    json!({"id": i, "groupId": 1, "characterId1": 1})
                }
            })
            .collect();
        std::fs::write(root.join("bonds.json"), serde_json::to_vec(&rows).unwrap()).unwrap();
        engine
            .ingest_master_data(root.to_str().unwrap(), "jp")
            .await
            .unwrap();
        assert_eq!(count("jp").await, (total as i64, (total - 2500) as i64));

        // Re-ingest replaces the region's rows only.
        std::fs::write(root.join("bonds.json"), r#"[{"id": 1, "groupId": 9}]"#).unwrap();
        engine
            .ingest_master_data(root.to_str().unwrap(), "en")
            .await
            .unwrap();
        engine
            .ingest_master_data(root.to_str().unwrap(), "jp")
            .await
            .unwrap();
        assert_eq!(count("jp").await, (1, 1));
        assert_eq!(count("en").await, (1, 1));

        // A truncated file fails and the transaction rolls back.
        std::fs::write(root.join("bonds.json"), r#"[{"id": 1}, {"id": 2"#).unwrap();
        assert!(engine
            .ingest_master_data(root.to_str().unwrap(), "jp")
            .await
            .is_err());
        assert_eq!(count("jp").await, (1, 1));
        // Same when the failure comes after whole batches were already
        // inserted: nothing of the truncated file may survive.
        let mut big = serde_json::to_vec(
            &(0..ROWS_PER_BATCH + 500)
                .map(|i| json!({"id": i, "groupId": 1}))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        big.truncate(big.len() - 3);
        std::fs::write(root.join("bonds.json"), &big).unwrap();
        assert!(engine
            .ingest_master_data(root.to_str().unwrap(), "jp")
            .await
            .is_err());
        assert_eq!(count("jp").await, (1, 1));

        // An empty array leaves existing rows in place (no DELETE is issued).
        std::fs::write(root.join("bonds.json"), "[]").unwrap();
        engine
            .ingest_master_data(root.to_str().unwrap(), "jp")
            .await
            .unwrap();
        assert_eq!(count("jp").await, (1, 1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn loads_schema_resolves_tables_and_skips_non_ingestable_files() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let engine = IngestionEngine::new(db).await.unwrap().with_concurrency(0);
        assert_eq!(engine.concurrency, 1);
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

    #[tokio::test]
    async fn resolves_master_table_gap_files_and_their_unique_keys() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let engine = IngestionEngine::new(db).await.unwrap();
        for (file, table) in [
            ("characterMissionV2s", "charactermissionv2s"),
            ("bondsHonorWords", "bondshonorwords"),
            (
                "worldBloomChapterRankingRewardRanges",
                "worldbloomchapterrankingrewardranges",
            ),
            ("resourceBoxDetails", "resourceboxdetails"),
            ("materials", "materials"),
            ("practiceTickets", "practicetickets"),
            ("skillPracticeTickets", "skillpracticetickets"),
            ("characterMissionV2ExJsons", "charactermissionv2exjsons"),
            ("characterMissionV2AreaItems", "charactermissionv2areaitems"),
        ] {
            assert_eq!(
                engine.resolve_table_name(file).as_deref(),
                Some(table),
                "{file}"
            );
        }
        // Dict-of-columns layout the row parser cannot read: must stay unmapped.
        assert!(engine
            .resolve_table_name("compactResourceBoxDetails")
            .is_none());

        let default_key = vec![vec!["id".to_string(), "server_region".to_string()]];
        for table in [
            "charactermissionv2s",
            "bondshonorwords",
            "worldbloomchapterrankingrewardranges",
            "materials",
            "practicetickets",
            "skillpracticetickets",
            "charactermissionv2exjsons",
            "charactermissionv2areaitems",
        ] {
            let (columns, keys) = &engine.schema.schema_map[table];
            assert!(columns.contains_key("game_id"), "{table}");
            assert_eq!(keys, &default_key, "{table}");
        }
        // Nuverse resourceBoxDetails rows have no id/seq, so no unique key is declared.
        let (columns, keys) = &engine.schema.schema_map["resourceboxdetails"];
        assert!(!columns.contains_key("game_id"));
        assert!(keys.is_empty());
    }

    #[tokio::test]
    async fn ingests_master_table_gap_fixtures_across_region_shapes() {
        use sea_orm::Statement;
        let mut opt = ConnectOptions::new("sqlite::memory:".to_string());
        opt.max_connections(1);
        let db = Database::connect(opt).await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE resourceboxdetails (resource_box_id INTEGER, resource_quantity INTEGER, \
             resource_id INTEGER, resource_box_purpose TEXT, resource_level INTEGER, \
             resource_type TEXT, server_region TEXT)",
        )
        .await
        .unwrap();
        db.execute_unprepared(
            "CREATE TABLE charactermissionv2s (game_id INTEGER, character_mission_type TEXT, \
             character_id INTEGER, parameter_group_id INTEGER, sentence TEXT, \
             progress_sentence TEXT, is_achievement_mission INTEGER, server_region TEXT)",
        )
        .await
        .unwrap();
        let engine = IngestionEngine::new(db.clone()).await.unwrap();
        let root = std::env::temp_dir().join(format!("haruki_ingest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        // CN shape: flat rows without id/seq, nullable resourceId/resourceLevel.
        std::fs::write(
            root.join("resourceBoxDetails.json"),
            r#"[{"resourceBoxId":1,"resourceQuantity":1,"resourceId":1,
                 "resourceBoxPurpose":"ad_reward","resourceLevel":null,
                 "resourceType":"ad_reward_random_box"},
                {"resourceBoxId":2,"resourceQuantity":100,"resourceId":null,
                 "resourceBoxPurpose":"shop_item","resourceLevel":null,"resourceType":"jewel"}]"#,
        )
        .unwrap();
        // Dict-of-columns sibling with no schema entry: skipped before parsing, so its
        // non-array layout must not fail the run.
        std::fs::write(
            root.join("compactResourceBoxDetails.json"),
            r#"{"__ENUM__":{"resourceType":["jewel"]},"resourceBoxId":[1],"resourceType":[0]}"#,
        )
        .unwrap();
        // CP key order first, Nuverse key order second.
        std::fs::write(
            root.join("characterMissionV2s.json"),
            r#"[{"id":1,"characterMissionType":"play_live","characterId":1,"parameterGroupId":1,
                 "sentence":"s","progressSentence":"p","isAchievementMission":false},
                {"characterId":2,"characterMissionType":"waiting_room_ex","id":2,
                 "isAchievementMission":true,"parameterGroupId":3,"progressSentence":"p",
                 "sentence":"s"}]"#,
        )
        .unwrap();
        engine
            .ingest_master_data(root.to_str().unwrap(), "cn")
            .await
            .unwrap();

        let query = |sql: String| {
            let db = db.clone();
            async move {
                db.query_one_raw(Statement::from_string(db.get_database_backend(), sql))
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        let row = query(
            "SELECT COUNT(*) AS n, SUM(resource_id IS NULL) AS null_ids, \
             SUM(resource_quantity) AS qty FROM resourceboxdetails WHERE server_region = 'cn'"
                .to_string(),
        )
        .await;
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
        assert_eq!(row.try_get::<i64>("", "null_ids").unwrap(), 1);
        assert_eq!(row.try_get::<i64>("", "qty").unwrap(), 101);
        let row = query(
            "SELECT COUNT(*) AS n, SUM(is_achievement_mission) AS achievements, \
             MAX(game_id) AS max_id FROM charactermissionv2s WHERE server_region = 'cn'"
                .to_string(),
        )
        .await;
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
        assert_eq!(row.try_get::<i64>("", "achievements").unwrap(), 1);
        assert_eq!(row.try_get::<i64>("", "max_id").unwrap(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn resolves_phase2_files_and_singular_y_stems_only() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let engine = IngestionEngine::new(db).await.unwrap();
        for (file, table) in [
            (
                "customProfileCharacterIconResources",
                "customprofilecharactericonresources",
            ),
            (
                "customProfileCollectionResources",
                "customprofilecollectionresources",
            ),
            ("customProfileEtcResources", "customprofileetcresources"),
            (
                "customProfileGeneralBackgroundResources",
                "customprofilegeneralbackgroundresources",
            ),
            (
                "customProfileMaterialResources",
                "customprofilematerialresources",
            ),
            (
                "customProfileMemberStandingPictureResources",
                "customprofilememberstandingpictureresources",
            ),
            (
                "customProfilePlayerInfoResources",
                "customprofileplayerinforesources",
            ),
            ("customProfileShapeResources", "customprofileshaperesources"),
            (
                "customProfileStoryBackgroundResources",
                "customprofilestorybackgroundresources",
            ),
            ("customProfileTextColors", "customprofiletextcolors"),
            ("customProfileTextFonts", "customprofiletextfonts"),
            (
                "customProfileUserInterfaceIconResources",
                "customprofileuserinterfaceiconresources",
            ),
            ("omikujis", "omikujis"),
            ("unitStoryEpisodeGroups", "unitstoryepisodegroups"),
            ("musicCategories", "musiccategories"),
            // A singular `…y` stem reaches its `…ies` table.
            ("musicCategory", "musiccategories"),
            ("cardRarity", "cardrarities"),
        ] {
            assert_eq!(
                engine.resolve_table_name(file).as_deref(),
                Some(table),
                "{file}"
            );
        }
        // No table exists for these, so the y->ies rule must not invent one.
        assert!(engine.resolve_table_name("streamingLiveCategory").is_none());
        assert!(engine
            .resolve_table_name("mysekaiStaminaRecovery")
            .is_none());
        // Single-object master: adding a `mysekaicolorfulpasses` table would bind it
        // through the `+es` rule and fail the file, so it must stay unmapped.
        assert!(engine.resolve_table_name("mysekaiColorfulPass").is_none());
        assert!(engine.resolve_table_name("compactCostume3ds").is_none());
        let default_key = vec![vec!["id".to_string(), "server_region".to_string()]];
        for table in [
            "customprofiletextfonts",
            "customprofilecollectionresources",
            "omikujis",
            "unitstoryepisodegroups",
            "musiccategories",
        ] {
            let (columns, keys) = &engine.schema.schema_map[table];
            assert!(columns.contains_key("game_id"), "{table}");
            assert_eq!(keys, &default_key, "{table}");
        }
        for column in [
            "world_bloom_support_deck_character_bonuses",
            "world_bloom_support_deck_master_rank_bonuses",
            "world_bloom_support_deck_skill_level_bonuses",
        ] {
            assert_eq!(
                engine.schema.schema_map["worldbloomsupportdeckbonuses"].0[column],
                "json.RawMessage"
            );
        }
    }

    /// Every schema_info table is reached by exactly one file stem per region
    /// (from the registry manifests in `testdata/master_file_stems.json`), and no
    /// unmapped stem is pulled onto an unrelated table by the plural rules.
    #[tokio::test]
    async fn every_schema_table_resolves_from_exactly_one_real_file_stem() {
        #[derive(serde::Deserialize)]
        struct Fixture {
            files: std::collections::BTreeMap<String, String>,
        }
        let fixture: Fixture =
            serde_json::from_str(include_str!("testdata/master_file_stems.json")).unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let engine = IngestionEngine::new(db).await.unwrap();
        // Tables whose file is absent from some regions.
        let partial: HashMap<&str, &[&str]> = HashMap::from([
            ("custommusicscoretags", &["jp"][..]),
            ("customprofilecharactericonresources", &["jp"][..]),
            ("customprofilematerialresources", &["jp"][..]),
            ("customprofileuserinterfaceiconresources", &["jp"][..]),
            ("musiccategories", &["jp"][..]),
            ("mysekaihousingcompetitions", &["jp", "tw", "kr", "cn"][..]),
            ("resourceboxdetails", &["tw", "kr", "cn"][..]),
        ]);
        for region in ["jp", "en", "tw", "kr", "cn"] {
            let mut resolved: HashMap<String, Vec<&str>> = HashMap::new();
            for (stem, regions) in &fixture.files {
                if !regions.split(',').any(|r| r == region) {
                    continue;
                }
                assert!(
                    !stem.starts_with("compact") || engine.resolve_table_name(stem).is_none(),
                    "{region}/{stem} is a dict-of-columns file and must stay unmapped"
                );
                if let Some(table) = engine.resolve_table_name(stem) {
                    resolved.entry(table).or_default().push(stem);
                }
            }
            for (table, stems) in &resolved {
                assert_eq!(stems.len(), 1, "{region}: {table} <- {stems:?}");
                let expected_stem = table.replace('_', "");
                assert_eq!(
                    stems[0].to_lowercase().replace('_', ""),
                    expected_stem,
                    "{region}: {table} resolved from an unexpected stem"
                );
            }
            for table in engine.schema.schema_map.keys() {
                let expected = partial
                    .get(table.as_str())
                    .is_none_or(|regions| regions.contains(&region));
                assert_eq!(
                    resolved.contains_key(table),
                    expected,
                    "{region}: {table} presence"
                );
            }
        }
    }

    #[tokio::test]
    async fn ingests_new_columns_and_phase2_tables_across_region_shapes() {
        use sea_orm::Statement;
        let mut opt = ConnectOptions::new("sqlite::memory:".to_string());
        opt.max_connections(1);
        let db = Database::connect(opt).await.unwrap();
        for ddl in [
            "CREATE TABLE playerframegroups (game_id INTEGER, seq INTEGER, name TEXT, \
             assetbundle_name TEXT, player_frame_type TEXT, edit_count INTEGER, \
             server_region TEXT)",
            "CREATE TABLE worldbloomsupportdeckbonuses (card_rarity_type TEXT, \
             world_bloom_support_deck_character_bonuses TEXT, \
             world_bloom_support_deck_master_rank_bonuses TEXT, \
             world_bloom_support_deck_skill_level_bonuses TEXT, server_region TEXT)",
            "CREATE TABLE customprofiletextfonts (game_id INTEGER, name TEXT, font_name TEXT, \
             assetbundle_name TEXT, server_region TEXT)",
            "CREATE TABLE musiccategories (game_id INTEGER, music_id INTEGER, \
             music_category_name TEXT, music_asset_variant_id INTEGER, published_at INTEGER, \
             server_region TEXT)",
        ] {
            db.execute_unprepared(ddl).await.unwrap();
        }
        let engine = IngestionEngine::new(db.clone()).await.unwrap();
        let root = std::env::temp_dir().join(format!("haruki_ingest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        // JP row carries the new keys, the second (Nuverse-shaped) row does not.
        std::fs::write(
            root.join("playerFrameGroups.json"),
            r#"[{"id":1,"seq":1,"name":"a","assetbundleName":"x","playerFrameType":"single","editCount":0},
                {"assetbundleName":"y","id":2,"name":"b","seq":2}]"#,
        )
        .unwrap();
        std::fs::write(
            root.join("worldBloomSupportDeckBonuses.json"),
            r#"[{"cardRarityType":"rarity_1",
                 "worldBloomSupportDeckCharacterBonuses":[{"bonusRate":5.5,"id":10101,
                   "worldBloomSupportDeckCharacterType":"specific"}],
                 "worldBloomSupportDeckMasterRankBonuses":[{"bonusRate":0.0,"id":10101,"masterRank":0}],
                 "worldBloomSupportDeckSkillLevelBonuses":[]}]"#,
        )
        .unwrap();
        std::fs::write(
            root.join("customProfileTextFonts.json"),
            r#"[{"id":1,"name":"ピュア１","fontName":"FOT-RodinNTLGPro-DB","assetbundleName":"custom_profile/font"},
                {"fontName":"NotoSansCJKtc-Medium","id":2,"name":"純真1"}]"#,
        )
        .unwrap();
        std::fs::write(
            root.join("musicCategories.json"),
            r#"[{"id":1,"musicId":1,"musicCategoryName":"mv"},
                {"id":2,"musicId":1,"musicCategoryName":"original","musicAssetVariantId":47701,
                 "publishedAt":1788404400000}]"#,
        )
        .unwrap();
        // Single-object sibling without a table: skipped, must not fail the run.
        std::fs::write(
            root.join("mysekaiColorfulPass.json"),
            r#"{"id":1,"expireDays":30}"#,
        )
        .unwrap();
        engine
            .ingest_master_data(root.to_str().unwrap(), "jp")
            .await
            .unwrap();

        let query = |sql: &'static str| {
            let db = db.clone();
            async move {
                db.query_one_raw(Statement::from_string(db.get_database_backend(), sql))
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        let row = query(
            "SELECT COUNT(*) AS n, SUM(player_frame_type IS NULL) AS null_types, \
             MIN(edit_count) AS min_edit FROM playerframegroups WHERE server_region = 'jp'",
        )
        .await;
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
        assert_eq!(row.try_get::<i64>("", "null_types").unwrap(), 1);
        assert_eq!(row.try_get::<i64>("", "min_edit").unwrap(), 0);
        let row = query(
            "SELECT world_bloom_support_deck_character_bonuses AS c, \
             world_bloom_support_deck_skill_level_bonuses AS s \
             FROM worldbloomsupportdeckbonuses WHERE server_region = 'jp'",
        )
        .await;
        let character: Value =
            serde_json::from_str(&row.try_get::<String>("", "c").unwrap()).unwrap();
        assert_eq!(character[0]["bonusRate"], json!(5.5));
        assert_eq!(row.try_get::<String>("", "s").unwrap(), "[]");
        let row = query(
            "SELECT COUNT(*) AS n, SUM(assetbundle_name IS NULL) AS null_ab \
             FROM customprofiletextfonts WHERE server_region = 'jp'",
        )
        .await;
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
        assert_eq!(row.try_get::<i64>("", "null_ab").unwrap(), 1);
        let row = query(
            "SELECT COUNT(*) AS n, MAX(music_asset_variant_id) AS variant, \
             MAX(published_at) AS published FROM musiccategories WHERE server_region = 'jp'",
        )
        .await;
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 2);
        assert_eq!(row.try_get::<i64>("", "variant").unwrap(), 47701);
        assert_eq!(
            row.try_get::<i64>("", "published").unwrap(),
            1_788_404_400_000
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

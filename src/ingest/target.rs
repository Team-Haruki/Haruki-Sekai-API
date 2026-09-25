//! One ingest target: its database, schema, table selection and the SQL that
//! writes a file, keeps the per-target state and checks a region.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use sea_orm::sea_query::{SimpleExpr, Value as SeaValue};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
    TransactionTrait,
};
use serde_json::Value;
use sha2::Digest as _;

use crate::config::{IngestTargetConfig, ServerRegion, TableSelection};
use crate::ingest_engine::{
    build_batch, collect_json_keys, insert_batch, map_target_columns, Batch, MasterSchema,
};
use crate::updater::prune::BUILTIN_PROTECTED_TABLES;

/// Bumped whenever value conversion, key normalization or the write SQL
/// changes, so every target re-ingests every table once (mapping hash).
pub const ENGINE_VERSION: &str = "1";

/// Staging table for the file being written (per connection, dropped at
/// commit and after each file).
const STAGE: &str = "_master_ingest_stage";
const ORD: &str = "__ingest_ord";
const RAW: &str = "__ingest_raw";

const STATE_DDL: &str = "\
CREATE TABLE IF NOT EXISTS master_ingest_state (
    target text NOT NULL, region text NOT NULL, file text NOT NULL,
    sha256 text NOT NULL, table_name text, mapping_hash text NOT NULL,
    rows bigint, missing_since text,
    ingested_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (target, region, file));
CREATE TABLE IF NOT EXISTS master_ingest_version (
    target text NOT NULL, region text NOT NULL,
    content_hash text, data_version text, git_commit text,
    started_at timestamptz, finished_at timestamptz,
    status text NOT NULL, error text, unknown_keys jsonb,
    PRIMARY KEY (target, region));
CREATE TABLE IF NOT EXISTS master_raw (
    region text NOT NULL, table_name text NOT NULL, key jsonb NOT NULL, raw jsonb NOT NULL,
    PRIMARY KEY (region, table_name, key));
CREATE TABLE IF NOT EXISTS master_ingest_allow_shrink (
    target text NOT NULL, region text NOT NULL, content_hash text NOT NULL,
    tables jsonb NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (target, region, content_hash));";

pub(crate) fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn stmt(sql: impl Into<String>, values: Vec<SeaValue>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Postgres, sql, values)
}

fn sha_hex(parts: &[&str]) -> String {
    let mut hasher = sha2::Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hex::encode(hasher.finalize())
}

/// Columns and unique indexes of a table as the target database has them.
#[derive(Debug, Clone, Default)]
pub struct TableShape {
    /// column -> `information_schema` data type
    pub columns: BTreeMap<String, String>,
    /// column -> default expression, for columns that have one
    pub defaults: BTreeMap<String, String>,
    pub unique_sets: Vec<BTreeSet<String>>,
}

/// How one table is written for this target.
#[derive(Debug, Clone)]
pub struct TablePlan {
    pub table: String,
    /// Schema columns that exist in the database, minus `server_region`
    /// (column -> schema type): what `build_batch` maps JSON keys onto.
    pub cols: HashMap<String, String>,
    /// Database data type per column of `cols`.
    pub db_types: BTreeMap<String, String>,
    /// Default expression per column of `cols` that has one: the staging
    /// table gets it too, so a key absent from a whole batch takes the
    /// column's default exactly as the old path's INSERT did.
    pub defaults: BTreeMap<String, String>,
    pub has_region: bool,
    /// Key columns (without `server_region`) of a unique index matching the
    /// schema key: rows are upserted. `None`: delete + insert.
    pub key: Option<Vec<String>>,
    pub mapping_hash: String,
}

/// A `master_ingest_state` row.
#[derive(Debug, Clone)]
pub struct StateRow {
    pub sha256: String,
    pub table_name: Option<String>,
    pub mapping_hash: String,
    pub rows: Option<i64>,
    pub missing_since: Option<String>,
}

/// A `master_ingest_version` row.
#[derive(Debug, Clone, Default)]
pub struct VersionRow {
    pub content_hash: Option<String>,
    pub data_version: Option<String>,
    pub status: String,
}

pub struct Target {
    pub name: String,
    pub cfg: IngestTargetConfig,
    pub db: DatabaseConnection,
    pub schema: MasterSchema,
    /// `None` = every schema table.
    selected: Option<HashSet<String>>,
    required: BTreeSet<String>,
    ddl_done: AtomicBool,
}

impl Target {
    /// Load the schema and resolve the table lists; the database is
    /// connected lazily so an unreachable target does not stop the others.
    pub async fn open(cfg: IngestTargetConfig, pool_size: u32) -> Result<Self> {
        let schema = MasterSchema::load(&cfg.schema)
            .await
            .with_context(|| format!("target {}: schema", cfg.name))?;
        let mut opts = ConnectOptions::new(&cfg.dsn);
        opts.max_connections(pool_size.max(2))
            .min_connections(0)
            .connect_lazy(true)
            .connect_timeout(Duration::from_secs(10))
            .acquire_timeout(Duration::from_secs(60))
            .sqlx_logging(false)
            // No prepared-statement cache: nearly every statement here is a
            // multi-row INSERT with up to 65535 parameters, and each distinct
            // one (per table, batch shape and tail) would stay cached on
            // every pooled connection, client side (~100 KB+ each, 100 per
            // connection) and in the server backend alike.
            .map_sqlx_postgres_opts(|o| o.statement_cache_capacity(0));
        let db = Database::connect(opts)
            .await
            .with_context(|| format!("target {}: database", cfg.name))?;
        Self::with_db(cfg, schema, db)
    }

    pub fn with_db(
        cfg: IngestTargetConfig,
        schema: MasterSchema,
        db: DatabaseConnection,
    ) -> Result<Self> {
        let resolve = |name: &str| -> Result<String> {
            let stem = name.strip_suffix(".json").unwrap_or(name);
            schema
                .table(stem)
                .map(|_| stem.to_string())
                .or_else(|| schema.resolve_table_name(stem))
                .ok_or_else(|| anyhow!("target {}: unknown table {name:?}", cfg.name))
        };
        let selected = match &cfg.tables {
            TableSelection::Keyword(k) if k == "all" => None,
            TableSelection::Keyword(k) => {
                bail!(
                    "target {}: tables must be `all` or a list, not {k:?}",
                    cfg.name
                )
            }
            TableSelection::List(list) => Some(
                list.iter()
                    .map(|t| resolve(t))
                    .collect::<Result<HashSet<_>>>()?,
            ),
        };
        let required_names: Vec<String> = match &cfg.required_tables {
            TableSelection::Keyword(k) if k == "default" => BUILTIN_PROTECTED_TABLES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            TableSelection::Keyword(k) => bail!(
                "target {}: required_tables must be `default` or a list, not {k:?}",
                cfg.name
            ),
            TableSelection::List(list) => list.clone(),
        };
        let mut required = BTreeSet::new();
        for name in &required_names {
            let table = resolve(name)?;
            if selected.as_ref().is_none_or(|s| s.contains(&table)) {
                required.insert(table);
            }
        }
        if !(0.0..=1.0).contains(&cfg.min_ratio) {
            bail!("target {}: min_ratio must be within 0..=1", cfg.name);
        }
        Ok(Self {
            name: cfg.name.clone(),
            cfg,
            db,
            schema,
            selected,
            required,
            ddl_done: AtomicBool::new(false),
        })
    }

    pub fn serves(&self, region: ServerRegion) -> bool {
        self.cfg.regions.contains(&region)
    }

    pub fn is_selected(&self, table: &str) -> bool {
        self.selected.as_ref().is_none_or(|s| s.contains(table))
    }

    pub fn required(&self) -> &BTreeSet<String> {
        &self.required
    }

    /// Resolve a table name or file stem against this target's schema.
    pub fn resolve(&self, name: &str) -> Option<String> {
        let stem = name.strip_suffix(".json").unwrap_or(name);
        if self.schema.table(stem).is_some() {
            return Some(stem.to_string());
        }
        self.schema.resolve_table_name(stem)
    }

    /// Whether `table` is protected from removal (prune protect list or
    /// `required_tables`).
    pub fn is_protected(&self, table: &str) -> bool {
        self.required.contains(table)
            || BUILTIN_PROTECTED_TABLES
                .iter()
                .any(|p| self.schema.resolve_table_name(p).as_deref() == Some(table))
    }

    /// Create the state tables once per process (serialized across
    /// processes sharing the database).
    pub async fn ensure_state_tables(&self) -> Result<()> {
        if self.ddl_done.load(Ordering::Acquire) {
            return Ok(());
        }
        let txn = self.db.begin().await?;
        txn.execute_raw(stmt(
            "SELECT pg_advisory_xact_lock(hashtext('master_ingest:ddl'))",
            vec![],
        ))
        .await?;
        txn.execute_unprepared(STATE_DDL).await?;
        txn.commit().await?;
        self.ddl_done.store(true, Ordering::Release);
        Ok(())
    }

    /// Try the per-(database, schema, region) advisory lock on a dedicated
    /// connection that is closed (releasing the lock) when the returned guard
    /// drops. The key names the database rather than the target, so two
    /// deployments whose targets point at the same tables under different
    /// names never write a region at the same time.
    pub async fn try_lock(&self, region: ServerRegion) -> Result<Option<RegionLock>> {
        let pool = self.db.get_postgres_connection_pool();
        let mut conn = pool.acquire().await.context("acquiring lock connection")?;
        conn.close_on_drop();
        let locked: bool = sea_orm::sqlx::query_scalar(
            "SELECT pg_try_advisory_lock(hashtext('master_ingest:' || current_database() \
             || ':' || current_schema() || ':' || $1))",
        )
        .bind(region.as_str())
        .fetch_one(&mut *conn)
        .await
        .context("advisory lock")?;
        Ok(locked.then_some(RegionLock { _conn: conn }))
    }

    /// Create the typed tables `tables` from the schema when they are
    /// missing (`create_tables: true` only).
    pub async fn create_missing_tables(&self, tables: &[String]) -> Result<()> {
        for table in tables {
            let Some(ddl) = typed_table_ddl(&self.schema, table) else {
                continue;
            };
            self.db
                .execute_unprepared(&ddl)
                .await
                .with_context(|| format!("creating table {table}"))?;
        }
        Ok(())
    }

    /// Columns and unique indexes of every table in the current schema.
    pub async fn introspect(&self) -> Result<HashMap<String, TableShape>> {
        let mut shapes: HashMap<String, TableShape> = HashMap::new();
        let rows = self
            .db
            .query_all_raw(stmt(
                "SELECT table_name::text AS t, column_name::text AS c, data_type::text AS d, \
                   column_default::text AS def \
                 FROM information_schema.columns WHERE table_schema = current_schema()",
                vec![],
            ))
            .await?;
        for row in rows {
            let table: String = row.try_get("", "t")?;
            let column: String = row.try_get("", "c")?;
            let default: Option<String> = row.try_get("", "def")?;
            let shape = shapes.entry(table).or_default();
            if let Some(default) = default {
                shape.defaults.insert(column.clone(), default);
            }
            shape.columns.insert(column, row.try_get("", "d")?);
        }
        let rows = self
            .db
            .query_all_raw(stmt(
                "SELECT c.relname::text AS t, string_agg(a.attname::text, ',') AS cols \
                 FROM pg_index i \
                 JOIN pg_class c ON c.oid = i.indrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) \
                 WHERE n.nspname = current_schema() AND i.indisunique AND i.indimmediate \
                   AND i.indpred IS NULL AND i.indexprs IS NULL \
                 GROUP BY c.relname, i.indexrelid",
                vec![],
            ))
            .await?;
        for row in rows {
            let table: String = row.try_get("", "t")?;
            let cols: String = row.try_get("", "cols")?;
            if let Some(shape) = shapes.get_mut(&table) {
                shape
                    .unique_sets
                    .push(cols.split(',').map(str::to_string).collect());
            }
        }
        Ok(shapes)
    }

    /// The write plan and mapping hash of `table`, given its database shape.
    pub fn plan(&self, table: &str, shape: &TableShape) -> Result<TablePlan> {
        let (schema_cols, unique_keys) = self
            .schema
            .table(table)
            .ok_or_else(|| anyhow!("table {table} is not in the schema"))?;
        let has_region = shape.columns.contains_key("server_region");
        let mut cols = HashMap::new();
        let mut db_types = BTreeMap::new();
        for (col, typ) in schema_cols {
            if col == "server_region" {
                continue;
            }
            if let Some(db_type) = shape.columns.get(col) {
                cols.insert(col.clone(), typ.clone());
                db_types.insert(col.clone(), db_type.clone());
            }
        }
        // Schema keys name the JSON `id` as `id`; the column is `game_id`.
        let key = unique_keys.first().and_then(|key| {
            let mapped: Vec<String> = key
                .iter()
                .map(|c| {
                    if c == "id" {
                        "game_id".to_string()
                    } else {
                        c.clone()
                    }
                })
                .collect();
            let set: BTreeSet<String> = mapped.iter().cloned().collect();
            let without_region: Vec<String> = mapped
                .into_iter()
                .filter(|c| c != "server_region")
                .collect();
            let usable = has_region == set.contains("server_region")
                && !without_region.is_empty()
                && without_region.iter().all(|c| cols.contains_key(c))
                && shape.unique_sets.contains(&set);
            usable.then_some(without_region)
        });
        let defaults: BTreeMap<String, String> = shape
            .defaults
            .iter()
            .filter(|(c, _)| db_types.contains_key(*c))
            .map(|(c, d)| (c.clone(), d.clone()))
            .collect();
        let columns_desc = db_types
            .iter()
            .map(|(c, t)| match defaults.get(c) {
                Some(d) => format!("{c}:{t}:{}:default={d}", cols[c]),
                None => format!("{c}:{t}:{}", cols[c]),
            })
            .collect::<Vec<_>>()
            .join(",");
        let key_desc = key.as_ref().map(|k| k.join(",")).unwrap_or_default();
        let mapping_hash = sha_hex(&[
            ENGINE_VERSION,
            table,
            self.schema.entry_fingerprint(table).unwrap_or(""),
            &columns_desc,
            &format!("region={has_region}"),
            &format!("key={key_desc}"),
            &format!("raw={}", self.cfg.raw),
            "selected",
        ]);
        Ok(TablePlan {
            table: table.to_string(),
            cols,
            db_types,
            defaults,
            has_region,
            key,
            mapping_hash,
        })
    }

    /// Mapping hash of a file that is not written (no table, legacy-skipped
    /// or outside the allow-list): only its state row is kept.
    pub fn skip_hash(table: Option<&str>, reason: &str) -> String {
        sha_hex(&[ENGINE_VERSION, "skip", reason, table.unwrap_or("")])
    }

    pub async fn load_state(
        &self,
        conn: &impl ConnectionTrait,
        region: ServerRegion,
    ) -> Result<HashMap<String, StateRow>> {
        let rows = conn
            .query_all_raw(stmt(
                "SELECT file, sha256, table_name, mapping_hash, rows, missing_since \
                 FROM master_ingest_state WHERE target = $1 AND region = $2",
                vec![self.name.clone().into(), region.as_str().into()],
            ))
            .await?;
        let mut out = HashMap::new();
        for row in rows {
            out.insert(
                row.try_get("", "file")?,
                StateRow {
                    sha256: row.try_get("", "sha256")?,
                    table_name: row.try_get("", "table_name")?,
                    mapping_hash: row.try_get("", "mapping_hash")?,
                    rows: row.try_get("", "rows")?,
                    missing_since: row.try_get("", "missing_since")?,
                },
            );
        }
        Ok(out)
    }

    pub async fn load_version(
        &self,
        conn: &impl ConnectionTrait,
        region: ServerRegion,
        for_update: bool,
    ) -> Result<Option<VersionRow>> {
        let sql = format!(
            "SELECT content_hash, data_version, status FROM master_ingest_version \
             WHERE target = $1 AND region = $2{}",
            if for_update { " FOR UPDATE" } else { "" }
        );
        let row = conn
            .query_one_raw(stmt(
                sql,
                vec![self.name.clone().into(), region.as_str().into()],
            ))
            .await?;
        row.map(|row| {
            Ok(VersionRow {
                content_hash: row.try_get("", "content_hash")?,
                data_version: row.try_get("", "data_version")?,
                status: row.try_get("", "status")?,
            })
        })
        .transpose()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_state(
        &self,
        conn: &impl ConnectionTrait,
        region: ServerRegion,
        file: &str,
        sha256: &str,
        table: Option<&str>,
        mapping_hash: &str,
        rows: Option<i64>,
        missing_since: Option<&str>,
    ) -> Result<()> {
        conn.execute_raw(stmt(
            "INSERT INTO master_ingest_state \
               (target, region, file, sha256, table_name, mapping_hash, rows, missing_since, ingested_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now()) \
             ON CONFLICT (target, region, file) DO UPDATE SET sha256 = EXCLUDED.sha256, \
               table_name = EXCLUDED.table_name, mapping_hash = EXCLUDED.mapping_hash, \
               rows = EXCLUDED.rows, missing_since = EXCLUDED.missing_since, \
               ingested_at = EXCLUDED.ingested_at",
            vec![
                self.name.clone().into(),
                region.as_str().into(),
                file.into(),
                sha256.into(),
                table.map(str::to_string).into(),
                mapping_hash.into(),
                rows.into(),
                missing_since.map(str::to_string).into(),
            ],
        ))
        .await?;
        Ok(())
    }

    pub async fn set_missing_since(
        &self,
        conn: &impl ConnectionTrait,
        region: ServerRegion,
        file: &str,
        missing_since: Option<&str>,
    ) -> Result<()> {
        conn.execute_raw(stmt(
            "UPDATE master_ingest_state SET missing_since = $4 \
             WHERE target = $1 AND region = $2 AND file = $3",
            vec![
                self.name.clone().into(),
                region.as_str().into(),
                file.into(),
                missing_since.map(str::to_string).into(),
            ],
        ))
        .await?;
        Ok(())
    }

    pub async fn delete_state(
        &self,
        conn: &impl ConnectionTrait,
        region: ServerRegion,
        file: &str,
    ) -> Result<()> {
        conn.execute_raw(stmt(
            "DELETE FROM master_ingest_state WHERE target = $1 AND region = $2 AND file = $3",
            vec![
                self.name.clone().into(),
                region.as_str().into(),
                file.into(),
            ],
        ))
        .await?;
        Ok(())
    }

    /// Record the start of a staged run (the recorded version is kept).
    pub async fn mark_staging(&self, region: ServerRegion) -> Result<()> {
        self.db
            .execute_raw(stmt(
                "INSERT INTO master_ingest_version (target, region, status, started_at) \
                 VALUES ($1, $2, 'staging', now()) \
                 ON CONFLICT (target, region) DO UPDATE SET status = 'staging', \
                   started_at = now(), finished_at = NULL, error = NULL",
                vec![self.name.clone().into(), region.as_str().into()],
            ))
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn write_version(
        &self,
        conn: &impl ConnectionTrait,
        region: ServerRegion,
        content_hash: &str,
        data_version: &str,
        git_commit: Option<&str>,
        started_at: chrono::DateTime<chrono::Utc>,
        note: Option<&str>,
        unknown_keys: &BTreeMap<String, BTreeSet<String>>,
    ) -> Result<()> {
        let unknown = if unknown_keys.is_empty() {
            None
        } else {
            Some(serde_json::to_value(unknown_keys)?)
        };
        conn.execute_raw(stmt(
            "INSERT INTO master_ingest_version (target, region, content_hash, data_version, \
               git_commit, started_at, finished_at, status, error, unknown_keys) \
             VALUES ($1, $2, $3, $4, $5, $6, now(), 'ok', $7, $8) \
             ON CONFLICT (target, region) DO UPDATE SET content_hash = EXCLUDED.content_hash, \
               data_version = EXCLUDED.data_version, git_commit = EXCLUDED.git_commit, \
               started_at = EXCLUDED.started_at, finished_at = EXCLUDED.finished_at, \
               status = 'ok', error = EXCLUDED.error, unknown_keys = EXCLUDED.unknown_keys",
            vec![
                self.name.clone().into(),
                region.as_str().into(),
                content_hash.into(),
                data_version.into(),
                git_commit.map(str::to_string).into(),
                started_at.into(),
                note.map(str::to_string).into(),
                SeaValue::Json(unknown.map(Box::new)),
            ],
        ))
        .await?;
        Ok(())
    }

    /// Record a failed run; the recorded (last good) version is kept.
    pub async fn record_failure(&self, region: ServerRegion, error: &str) -> Result<()> {
        self.db
            .execute_raw(stmt(
                "INSERT INTO master_ingest_version (target, region, status, error, finished_at) \
                 VALUES ($1, $2, 'failed', $3, now()) \
                 ON CONFLICT (target, region) DO UPDATE SET status = 'failed', \
                   error = EXCLUDED.error, finished_at = EXCLUDED.finished_at",
                vec![
                    self.name.clone().into(),
                    region.as_str().into(),
                    error.into(),
                ],
            ))
            .await?;
        Ok(())
    }

    /// Tables allowed to shrink below `min_ratio` for `content_hash`
    /// (config plus stored overrides).
    pub async fn allowed_shrink(
        &self,
        conn: &impl ConnectionTrait,
        region: ServerRegion,
        content_hash: &str,
    ) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        for entry in &self.cfg.allow_shrink {
            if entry.content_hash == content_hash {
                out.extend(entry.tables.iter().filter_map(|t| self.resolve(t)));
            }
        }
        let row = conn
            .query_one_raw(stmt(
                "SELECT tables FROM master_ingest_allow_shrink \
                 WHERE target = $1 AND region = $2 AND content_hash = $3",
                vec![
                    self.name.clone().into(),
                    region.as_str().into(),
                    content_hash.into(),
                ],
            ))
            .await?;
        if let Some(row) = row {
            let tables: Value = row.try_get("", "tables")?;
            for t in tables.as_array().into_iter().flatten() {
                if let Some(table) = t.as_str().and_then(|t| self.resolve(t)) {
                    out.insert(table);
                }
            }
        }
        Ok(out)
    }

    /// Store an allow-shrink override (merged with an existing one).
    pub async fn store_allow_shrink(
        &self,
        region: ServerRegion,
        content_hash: &str,
        tables: &[String],
    ) -> Result<()> {
        self.ensure_state_tables().await?;
        let tables = serde_json::to_value(tables)?;
        self.db
            .execute_raw(stmt(
                "INSERT INTO master_ingest_allow_shrink (target, region, content_hash, tables) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT (target, region, content_hash) \
                 DO UPDATE SET tables = master_ingest_allow_shrink.tables || EXCLUDED.tables",
                vec![
                    self.name.clone().into(),
                    region.as_str().into(),
                    content_hash.into(),
                    SeaValue::Json(Some(Box::new(tables))),
                ],
            ))
            .await?;
        Ok(())
    }

    /// Number of rows `table` holds for `region`.
    pub async fn count_rows(
        &self,
        conn: &impl ConnectionTrait,
        table: &str,
        region: ServerRegion,
        has_region: bool,
    ) -> Result<i64> {
        let (sql, values) = if has_region {
            (
                format!(
                    "SELECT count(*) AS n FROM {} WHERE server_region = $1",
                    quote(table)
                ),
                vec![region.as_str().into()],
            )
        } else {
            (
                format!("SELECT count(*) AS n FROM {}", quote(table)),
                vec![],
            )
        };
        let row = conn
            .query_one_raw(stmt(sql, values))
            .await?
            .ok_or_else(|| anyhow!("count returned no row"))?;
        Ok(row.try_get("", "n")?)
    }

    /// Whether `table` holds at least one row for `region`.
    pub async fn has_rows(
        &self,
        conn: &impl ConnectionTrait,
        table: &str,
        region: ServerRegion,
        has_region: bool,
    ) -> Result<bool> {
        let sql = if has_region {
            format!(
                "SELECT EXISTS (SELECT 1 FROM {} WHERE server_region = $1) AS e",
                quote(table)
            )
        } else {
            format!("SELECT EXISTS (SELECT 1 FROM {}) AS e", quote(table))
        };
        let values = if has_region {
            vec![region.as_str().into()]
        } else {
            vec![]
        };
        let row = conn
            .query_one_raw(stmt(sql, values))
            .await?
            .ok_or_else(|| anyhow!("EXISTS returned no row"))?;
        Ok(row.try_get("", "e")?)
    }

    /// Delete a removed table's rows for the region (typed and raw).
    pub async fn clear_table(
        &self,
        conn: &impl ConnectionTrait,
        table: &str,
        region: ServerRegion,
        has_region: bool,
    ) -> Result<()> {
        if has_region {
            conn.execute_raw(stmt(
                format!("DELETE FROM {} WHERE server_region = $1", quote(table)),
                vec![region.as_str().into()],
            ))
            .await?;
        } else {
            conn.execute_unprepared(&format!("DELETE FROM {}", quote(table)))
                .await?;
        }
        conn.execute_raw(stmt(
            "DELETE FROM master_raw WHERE region = $1 AND table_name = $2",
            vec![region.as_str().into(), table.into()],
        ))
        .await?;
        Ok(())
    }
}

/// Holds a session advisory lock; dropping it closes the connection, which
/// releases the lock on every path (including a panic).
pub struct RegionLock {
    _conn: sea_orm::sqlx::pool::PoolConnection<sea_orm::sqlx::Postgres>,
}

/// What one file contributed: its row count and the JSON keys that map to no
/// column.
#[derive(Debug, Default)]
pub struct FileWrite {
    pub rows: i64,
    pub unknown_keys: BTreeSet<String>,
}

/// Stage `batches` of one file, then bring the table's rows for the region
/// (and `master_raw` when `raw`) in line with it, on `conn` (inside the
/// caller's transaction). The parse outcome is awaited before anything is
/// applied: a failed or unverified parse leaves the table untouched.
pub async fn write_file(
    conn: &impl ConnectionTrait,
    plan: &TablePlan,
    region: ServerRegion,
    raw: bool,
    batches: &mut tokio::sync::mpsc::Receiver<std::sync::Arc<Vec<Value>>>,
    parsed: tokio::sync::oneshot::Receiver<Result<(), String>>,
) -> Result<FileWrite> {
    let table = quote(&plan.table);
    let cols: Vec<&String> = plan.db_types.keys().collect();
    let col_list: String = cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
    let select_cols = if col_list.is_empty() {
        String::new()
    } else {
        format!("{col_list}, ")
    };
    conn.execute_unprepared(&format!("DROP TABLE IF EXISTS pg_temp.{STAGE}"))
        .await?;
    conn.execute_unprepared(&format!(
        "CREATE TEMP TABLE {STAGE} ON COMMIT DROP AS SELECT {select_cols}\
         NULL::bigint AS {ORD}, NULL::jsonb AS {RAW} FROM {table} WITH NO DATA"
    ))
    .await
    .with_context(|| format!("staging {}", plan.table))?;
    for (col, default) in &plan.defaults {
        conn.execute_unprepared(&format!(
            "ALTER TABLE pg_temp.{STAGE} ALTER COLUMN {} SET DEFAULT {default}",
            quote(col)
        ))
        .await
        .with_context(|| format!("staging default of {}.{col}", plan.table))?;
    }

    let mut out = FileWrite::default();
    let mut ord: i64 = 0;
    while let Some(rows) = batches.recv().await {
        let Batch {
            mut column_names,
            rows: typed,
        } = build_batch(&rows, &plan.table, &plan.cols, region.as_str(), false);
        if typed.is_empty() {
            continue;
        }
        let keys = collect_json_keys(&rows);
        let mapped: HashSet<String> = map_target_columns(&keys, &plan.cols)
            .into_iter()
            .map(|m| m.json_key)
            .collect();
        out.unknown_keys
            .extend(keys.into_iter().filter(|k| !mapped.contains(k)));
        column_names.push(ORD.to_string());
        if raw {
            column_names.push(RAW.to_string());
        }
        let objects = rows.iter().filter(|v| v.is_object());
        let staged: Vec<Vec<SimpleExpr>> = typed
            .into_iter()
            .zip(objects)
            .map(|(mut row, obj)| {
                row.push(SeaValue::BigInt(Some(ord)).into());
                ord += 1;
                if raw {
                    row.push(SeaValue::Json(Some(Box::new(obj.clone()))).into());
                }
                row
            })
            .collect();
        // This receiver's handle on the parsed rows: once the other targets
        // are done with the batch too, it is freed before the INSERT is built.
        drop(rows);
        insert_batch(
            conn,
            STAGE,
            Batch {
                column_names,
                rows: staged,
            },
        )
        .await
        .with_context(|| format!("staging rows of {}", plan.table))?;
    }
    match parsed.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => bail!("{e}"),
        Err(_) => bail!("parser ended without an outcome"),
    }
    out.rows = ord;
    // Planner statistics for the anti-join and upsert below.
    conn.execute_unprepared(&format!("ANALYZE pg_temp.{STAGE}"))
        .await?;

    let region_param = || vec![SeaValue::from(region.as_str())];
    let insert_cols = if plan.has_region {
        format!("{select_cols}server_region")
    } else {
        col_list.clone()
    };
    let insert_vals = if plan.has_region {
        format!("{select_cols}$1")
    } else {
        col_list.clone()
    };
    let insert_params = || {
        if plan.has_region {
            region_param()
        } else {
            vec![]
        }
    };
    match &plan.key {
        Some(key) => {
            let not_null = key
                .iter()
                .map(|k| format!("s.{} IS NOT NULL", quote(k)))
                .collect::<Vec<_>>()
                .join(" AND ");
            let key_match = key
                .iter()
                .map(|k| format!("s.{0} = t.{0}", quote(k)))
                .collect::<Vec<_>>()
                .join(" AND ");
            let t_null = key
                .iter()
                .map(|k| format!("t.{} IS NULL", quote(k)))
                .collect::<Vec<_>>()
                .join(" OR ");
            let region_filter = if plan.has_region {
                "t.server_region = $1 AND "
            } else {
                ""
            };
            // Rows whose key left the file, and every row with a NULL key
            // (those never conflict, so they are replaced wholesale).
            conn.execute_raw(stmt(
                format!(
                    "DELETE FROM {table} AS t WHERE {region_filter}({t_null} OR NOT EXISTS \
                     (SELECT 1 FROM {STAGE} s WHERE {key_match}))"
                ),
                insert_params(),
            ))
            .await
            .with_context(|| format!("deleting stale rows of {}", plan.table))?;
            let conflict = if plan.has_region {
                format!(
                    "{}, server_region",
                    key.iter().map(|k| quote(k)).collect::<Vec<_>>().join(", ")
                )
            } else {
                key.iter().map(|k| quote(k)).collect::<Vec<_>>().join(", ")
            };
            let others: Vec<&&String> = cols.iter().filter(|c| !key.contains(c)).collect();
            let action = if others.is_empty() {
                "DO NOTHING".to_string()
            } else {
                let set = others
                    .iter()
                    .map(|c| format!("{0} = EXCLUDED.{0}", quote(c)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let cmp = |side: &str| {
                    others
                        .iter()
                        .map(|c| {
                            let col = format!("{side}.{}", quote(c));
                            if plan.db_types[c.as_str()] == "json" {
                                format!("{col}::text")
                            } else {
                                col
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!(
                    "DO UPDATE SET {set} WHERE ({}) IS DISTINCT FROM ({})",
                    cmp("t"),
                    cmp("EXCLUDED")
                )
            };
            let s_cols = select_with_prefix(&cols, "s");
            let s_vals = if plan.has_region {
                format!("{s_cols}$1")
            } else {
                s_cols.trim_end_matches(", ").to_string()
            };
            conn.execute_raw(stmt(
                format!(
                    "INSERT INTO {table} AS t ({insert_cols}) SELECT {s_vals} FROM {STAGE} s \
                     WHERE {not_null} ORDER BY s.{ORD} ON CONFLICT ({conflict}) {action}"
                ),
                insert_params(),
            ))
            .await
            .with_context(|| format!("upserting {}", plan.table))?;
            conn.execute_raw(stmt(
                format!(
                    "INSERT INTO {table} ({insert_cols}) SELECT {s_vals} FROM {STAGE} s \
                     WHERE NOT ({not_null}) ORDER BY s.{ORD}"
                ),
                insert_params(),
            ))
            .await
            .with_context(|| format!("inserting NULL-key rows of {}", plan.table))?;
        }
        None => {
            if plan.has_region {
                conn.execute_raw(stmt(
                    format!("DELETE FROM {table} WHERE server_region = $1"),
                    region_param(),
                ))
                .await?;
            } else {
                conn.execute_unprepared(&format!("DELETE FROM {table}"))
                    .await?;
            }
            conn.execute_raw(stmt(
                format!(
                    "INSERT INTO {table} ({insert_cols}) SELECT {insert_vals} FROM {STAGE} \
                     ORDER BY {ORD}"
                ),
                insert_params(),
            ))
            .await
            .with_context(|| format!("inserting {}", plan.table))?;
        }
    }

    if raw {
        let key_expr = match &plan.key {
            Some(key) => {
                let pairs = key
                    .iter()
                    .map(|k| format!("'{}', s.{}", k.replace('\'', "''"), quote(k)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let not_null = key
                    .iter()
                    .map(|k| format!("s.{} IS NOT NULL", quote(k)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                format!(
                    "CASE WHEN {not_null} THEN jsonb_build_object({pairs}) \
                     ELSE jsonb_build_object('_ord', s.{ORD}) END"
                )
            }
            None => format!("jsonb_build_object('_ord', s.{ORD})"),
        };
        let params = || vec![SeaValue::from(region.as_str()), plan.table.as_str().into()];
        conn.execute_raw(stmt(
            format!(
                "DELETE FROM master_raw m WHERE m.region = $1 AND m.table_name = $2 AND NOT EXISTS \
                 (SELECT 1 FROM {STAGE} s WHERE {key_expr} = m.key)"
            ),
            params(),
        ))
        .await
        .context("deleting stale raw rows")?;
        conn.execute_raw(stmt(
            format!(
                "INSERT INTO master_raw AS m (region, table_name, key, raw) \
                 SELECT $1, $2, {key_expr}, s.{RAW} FROM {STAGE} s \
                 ON CONFLICT (region, table_name, key) DO UPDATE SET raw = EXCLUDED.raw \
                 WHERE m.raw IS DISTINCT FROM EXCLUDED.raw"
            ),
            params(),
        ))
        .await
        .context("upserting raw rows")?;
    }
    conn.execute_unprepared(&format!("DROP TABLE IF EXISTS pg_temp.{STAGE}"))
        .await?;
    Ok(out)
}

fn select_with_prefix(cols: &[&String], prefix: &str) -> String {
    cols.iter()
        .map(|c| format!("{prefix}.{}, ", quote(c)))
        .collect::<String>()
}

/// `CREATE TABLE IF NOT EXISTS` for a typed table from the schema, with the
/// same shape the EntGo migrations give it (bigserial `id`, unique key).
pub fn typed_table_ddl(schema: &MasterSchema, table: &str) -> Option<String> {
    let (cols, keys) = schema.table(table)?;
    let mut names: Vec<&String> = cols.keys().collect();
    names.sort();
    let mut defs = vec!["id bigserial PRIMARY KEY".to_string()];
    for name in names {
        let sql_type = match cols[name].as_str() {
            "int64" | "int32" | "int" => "bigint",
            "float64" | "float32" | "float" => "double precision",
            "bool" => "boolean",
            "string" => "text",
            _ => "jsonb",
        };
        let not_null = if name == "server_region" {
            " NOT NULL"
        } else {
            ""
        };
        defs.push(format!("{} {sql_type}{not_null}", quote(name)));
    }
    let mut ddl = format!(
        "CREATE TABLE IF NOT EXISTS {} ({});",
        quote(table),
        defs.join(", ")
    );
    if let Some(key) = keys.first() {
        let key_cols = key
            .iter()
            .map(|c| quote(if c == "id" { "game_id" } else { c }))
            .collect::<Vec<_>>()
            .join(", ");
        ddl.push_str(&format!(
            " CREATE UNIQUE INDEX IF NOT EXISTS {} ON {} ({key_cols});",
            quote(&format!("{table}_ingest_key")),
            quote(table)
        ));
    }
    Some(ddl)
}

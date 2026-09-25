//! Ingest role tests. The PostgreSQL ones are `#[ignore]`d: set
//! `HARUKI_TEST_INGEST_DSN` to a scratch database whose user may create
//! databases (each test creates and drops its own), then
//! `cargo test ingest:: -- --ignored`. `HARUKI_TEST_INGEST_MASTER_DIR`
//! additionally runs the equivalence check over a full master directory.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use sha2::Digest as _;

use super::run::TargetOutcome;
use super::target::{typed_table_ddl, TableShape};
use super::{Ingester, Target};
use crate::api::internal::{manifest_content_hash, MasterManifest, MasterManifestFile};
use crate::config::{IngestConfig, IngestTargetConfig, ServerRegion, TableSelection};
use crate::ingest_engine::{IngestionEngine, MasterSchema};

const FIXTURE: &str = "src/testdata/ingest_fixture";
const JP: ServerRegion = ServerRegion::Jp;

// ---------------------------------------------------------------- registry mock

#[derive(Default)]
struct MockState {
    blob_store: String,
    current: HashMap<String, MasterManifest>,
    blobs: HashMap<String, Vec<u8>>,
    hits: HashMap<String, usize>,
    /// Digests answered 404 (a pruned manifest's blobs).
    gone: HashSet<String>,
    /// Becomes `current` of jp when a gone digest is requested.
    on_gone: Option<MasterManifest>,
}

type Mock = Arc<parking_lot::Mutex<MockState>>;

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

async fn mock_registry(blob_store: &str) -> (String, Mock) {
    let mock: Mock = Arc::new(parking_lot::Mutex::new(MockState {
        blob_store: blob_store.to_string(),
        ..Default::default()
    }));
    let router = Router::new()
        .route(
            "/health",
            get(|State(m): State<Mock>| async move {
                axum::Json(serde_json::json!({"blobStore": m.lock().blob_store}))
            }),
        )
        .route(
            "/v1/master/{region}/current",
            get(
                |State(m): State<Mock>, Path(region): Path<String>| async move {
                    match m.lock().current.get(&region) {
                        Some(c) => axum::Json(c.clone()).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
        )
        .route(
            "/v1/master/{region}/blob/{sha}",
            get(
                |State(m): State<Mock>, Path((_r, sha)): Path<(String, String)>| async move {
                    let mut m = m.lock();
                    if m.gone.contains(&sha) {
                        if let Some(next) = m.on_gone.take() {
                            m.current.insert("jp".into(), next);
                        }
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    *m.hits.entry(sha.clone()).or_default() += 1;
                    match m.blobs.get(&sha) {
                        Some(b) => Response::new(axum::body::Body::from(b.clone())),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
        )
        .with_state(mock.clone());
    (serve(router).await, mock)
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// A manifest for `files` (not yet current).
fn manifest(mock: &Mock, files: &BTreeMap<String, Vec<u8>>, data_version: &str) -> MasterManifest {
    let mut entries = Vec::new();
    let mut m = mock.lock();
    for (name, bytes) in files {
        let digest = sha(bytes);
        m.blobs.insert(digest.clone(), bytes.clone());
        entries.push(MasterManifestFile {
            name: name.clone(),
            size: bytes.len() as u64,
            sha256: digest,
        });
    }
    MasterManifest {
        server: "jp".into(),
        app_version: "6.0.0".into(),
        app_hash: "h".into(),
        data_version: data_version.into(),
        asset_version: data_version.into(),
        asset_hash: "a".into(),
        cdn_version: 0,
        generated_at: "2026-09-25T00:00:00Z".into(),
        content_hash: manifest_content_hash(&entries),
        git_commit: Some("0123abcd".into()),
        files: entries,
    }
}

fn publish(mock: &Mock, files: &BTreeMap<String, Vec<u8>>, data_version: &str) -> MasterManifest {
    let m = manifest(mock, files, data_version);
    mock.lock().current.insert("jp".into(), m.clone());
    m
}

fn hits(mock: &Mock, bytes: &[u8]) -> usize {
    mock.lock().hits.get(&sha(bytes)).copied().unwrap_or(0)
}

fn fixture() -> BTreeMap<String, Vec<u8>> {
    fixture_from(FIXTURE)
}

fn fixture_from(dir: &str) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            files.insert(
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&path).unwrap(),
            );
        }
    }
    files
}

/// Replace one file's rows.
fn with_rows(
    files: &BTreeMap<String, Vec<u8>>,
    name: &str,
    edit: impl FnOnce(&mut Vec<serde_json::Value>),
) -> BTreeMap<String, Vec<u8>> {
    let mut files = files.clone();
    let mut rows: Vec<serde_json::Value> = serde_json::from_slice(&files[name]).unwrap();
    edit(&mut rows);
    files.insert(name.to_string(), serde_json::to_vec(&rows).unwrap());
    files
}

// ---------------------------------------------------------------- config

fn target_cfg(name: &str, dsn: &str) -> IngestTargetConfig {
    let mut cfg: IngestTargetConfig =
        serde_yaml::from_str(&format!("name: {name}\ndsn: \"{dsn}\"\n")).unwrap();
    cfg.regions = vec![JP];
    cfg.required_tables = TableSelection::List(vec!["cards".into()]);
    cfg
}

fn ingest_cfg(registry_url: &str, stage_tables: usize) -> IngestConfig {
    let mut cfg: IngestConfig =
        serde_yaml::from_str(&format!("registry_url: \"{registry_url}\"\n")).unwrap();
    cfg.webhook_token = "hook-token".into();
    cfg.stage_tables = stage_tables;
    cfg
}

async fn ingester(
    registry_url: &str,
    stage_tables: usize,
    targets: Vec<IngestTargetConfig>,
) -> Arc<Ingester> {
    let mut cfg = ingest_cfg(registry_url, stage_tables);
    cfg.targets = targets.clone();
    let mut opened = Vec::new();
    for t in targets {
        opened.push(Arc::new(Target::open(t, 5).await.unwrap()));
    }
    Arc::new(Ingester::with_targets(cfg, opened))
}

fn outcome<'a>(outcomes: &'a [(String, TargetOutcome)], target: &str) -> &'a TargetOutcome {
    &outcomes.iter().find(|(n, _)| n == target).unwrap().1
}

fn written(o: &TargetOutcome) -> (usize, usize, bool) {
    match o {
        TargetOutcome::Ingested {
            written,
            removed,
            staged,
            ..
        } => (*written, *removed, *staged),
        other => panic!("expected Ingested, got {other:?}"),
    }
}

// ---------------------------------------------------------------- databases

struct Dbs {
    admin: DatabaseConnection,
    base: String,
    names: Vec<String>,
}

impl Dbs {
    async fn new() -> Self {
        let base = std::env::var("HARUKI_TEST_INGEST_DSN")
            .unwrap_or_else(|_| "postgres://haruki:sekai@localhost:5432/ingest_test".to_string());
        let admin = Database::connect(&base).await.unwrap();
        Self {
            admin,
            base,
            names: Vec::new(),
        }
    }

    /// A fresh database with every typed table of the schema.
    async fn create(&mut self, prefix: &str) -> (String, DatabaseConnection) {
        let name = format!("{prefix}_{}", uuid::Uuid::new_v4().simple());
        self.admin
            .execute_unprepared(&format!("CREATE DATABASE {name}"))
            .await
            .unwrap();
        self.names.push(name.clone());
        let (head, _) = self.base.rsplit_once('/').unwrap();
        let dsn = format!("{head}/{name}");
        let db = Database::connect(&dsn).await.unwrap();
        let schema = MasterSchema::load("schema_info.json").await.unwrap();
        let mut ddl = String::new();
        for table in schema.table_names() {
            ddl.push_str(&typed_table_ddl(&schema, table).unwrap());
        }
        db.execute_unprepared(&ddl).await.unwrap();
        (dsn, db)
    }

    async fn drop_all(self) {
        for name in self.names {
            let _ = self
                .admin
                .execute_unprepared(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
                .await;
        }
    }
}

async fn scalar_i64(db: &DatabaseConnection, sql: &str) -> i64 {
    let row = db
        .query_one_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index::<i64>(0).unwrap()
}

/// Row count and digest of a table's rows for jp (serial `id` excluded).
async fn digest(db: &DatabaseConnection, table: &str) -> (i64, String) {
    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            format!(
                "SELECT count(*) AS n, coalesce(md5(string_agg(r, E'\\n' ORDER BY r)), '') AS h \
                 FROM (SELECT (to_jsonb(t) - 'id')::text AS r FROM \"{table}\" t \
                       WHERE server_region = 'jp') x"
            ),
        ))
        .await
        .unwrap()
        .unwrap();
    (row.try_get("", "n").unwrap(), row.try_get("", "h").unwrap())
}

/// Tables the files map to (for this schema).
fn mapped_tables(files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, String> {
    let schema =
        MasterSchema::parse(&std::fs::read_to_string("schema_info.json").unwrap()).unwrap();
    files
        .keys()
        .filter_map(|f| {
            let stem = f.strip_suffix(".json").unwrap();
            schema
                .resolve_table_name(stem)
                .filter(|t| !crate::ingest_engine::is_legacy_skipped_table(t))
                .map(|t| (f.clone(), t))
        })
        .collect()
}

async fn old_path_ingest(db: &DatabaseConnection, files: &BTreeMap<String, Vec<u8>>) {
    let dir = std::env::temp_dir().join(format!("haruki_ingest_old_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    IngestionEngine::new(db.clone())
        .await
        .unwrap()
        .ingest_master_data(dir.to_str().unwrap(), "jp")
        .await
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

async fn assert_same_as_old(
    old: &DatabaseConnection,
    new: &DatabaseConnection,
    tables: impl IntoIterator<Item = &String>,
) {
    for table in tables {
        let expected = digest(old, table).await;
        assert!(expected.0 > 0, "{table}: the old path wrote no rows");
        assert_eq!(digest(new, table).await, expected, "{table}");
    }
}

// ---------------------------------------------------------------- PostgreSQL tests

/// The new role, ingesting a real-data fixture into two targets (one staged
/// full target with raw rows, one allow-list target in one transaction),
/// produces exactly the rows of the old in-sync path; every file is fetched
/// once however many targets need it; an update rewrites only its file.
#[tokio::test]
#[ignore] // Requires PostgreSQL (HARUKI_TEST_INGEST_DSN)
async fn two_targets_match_the_old_path_and_fetch_each_file_once() {
    let mut dbs = Dbs::new().await;
    let (_, old) = dbs.create("ing_old").await;
    let (dsn_a, db_a) = dbs.create("ing_a").await;
    let (dsn_b, db_b) = dbs.create("ing_b").await;
    let (url, mock) = mock_registry("pg").await;
    let files = fixture();
    let mapped = mapped_tables(&files);
    assert!(mapped.len() >= 12, "{mapped:?}");
    let v1 = publish(&mock, &files, "1.0.0.1");

    let mut a = target_cfg("full", &dsn_a);
    a.raw = true;
    let mut b = target_cfg("subset", &dsn_b);
    let subset = [
        "cards",
        "events",
        "musics",
        "ngWords",
        "cardRarities",
        "resourceBoxes",
    ];
    b.tables = TableSelection::List(subset.iter().map(|s| s.to_string()).collect());
    // `full` rewrites more than 8 tables (staged), `subset` fewer (one txn).
    let ing = ingester(&url, 8, vec![a, b]).await;

    let outcomes = ing.reconcile_region(JP).await;
    assert_eq!(written(outcome(&outcomes, "full")), (mapped.len(), 0, true));
    assert_eq!(
        written(outcome(&outcomes, "subset")),
        (subset.len(), 0, false)
    );
    for (file, bytes) in &files {
        let expected = usize::from(mapped.contains_key(file));
        assert_eq!(hits(&mock, bytes), expected, "{file} fetched");
    }

    old_path_ingest(&old, &files).await;
    assert_same_as_old(&old, &db_a, mapped.values()).await;
    let subset_tables: Vec<String> = mapped
        .values()
        .filter(|t| {
            [
                "cards",
                "events",
                "musics",
                "ngwords",
                "cardrarities",
                "resourceboxes",
            ]
            .contains(&t.as_str())
        })
        .cloned()
        .collect();
    assert_eq!(subset_tables.len(), subset.len());
    assert_same_as_old(&old, &db_b, subset_tables.iter()).await;
    assert_eq!(digest(&db_b, "skills").await.0, 0, "not selected");

    // Raw rows: one per typed row for `full`, none for `subset`.
    let mut typed_rows = 0;
    for table in mapped.values() {
        typed_rows += digest(&db_a, table).await.0;
    }
    assert_eq!(
        scalar_i64(&db_a, "SELECT count(*) FROM master_raw").await,
        typed_rows
    );
    assert_eq!(
        scalar_i64(&db_b, "SELECT count(*) FROM master_raw").await,
        0
    );
    for db in [&db_a, &db_b] {
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT content_hash, data_version, status FROM master_ingest_version",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_get::<String>("", "content_hash").unwrap(),
            v1.content_hash
        );
        assert_eq!(row.try_get::<String>("", "status").unwrap(), "ok");
    }
    // State rows cover every manifest file, per target.
    assert_eq!(
        scalar_i64(
            &db_a,
            "SELECT count(*) FROM master_ingest_state WHERE target = 'full'"
        )
        .await,
        files.len() as i64
    );

    // Skip unchanged: nothing is fetched or written again.
    let again = ing.reconcile_region(JP).await;
    for name in ["full", "subset"] {
        assert!(
            matches!(outcome(&again, name), TargetOutcome::UpToDate { .. }),
            "{again:?}"
        );
    }
    assert_eq!(hits(&mock, &files["cards.json"]), 1);

    // An update of one file: only it is fetched and written, in one
    // transaction; rows removed from the file leave the table.
    let v2_files = with_rows(&files, "cards.json", |rows| {
        rows.pop();
        rows[0]["prefix"] = serde_json::json!("changed prefix");
        rows[1]["brandNewField"] = serde_json::json!(1);
    });
    publish(&mock, &v2_files, "1.0.0.2");
    let outcomes = ing.reconcile_region(JP).await;
    assert_eq!(written(outcome(&outcomes, "full")), (1, 0, false));
    assert_eq!(written(outcome(&outcomes, "subset")), (1, 0, false));
    assert_eq!(hits(&mock, &v2_files["cards.json"]), 1);
    assert_eq!(hits(&mock, &files["events.json"]), 1);
    old_path_ingest(&old, &v2_files).await;
    assert_same_as_old(&old, &db_a, mapped.values()).await;
    assert_same_as_old(&old, &db_b, subset_tables.iter()).await;
    // The unknown key is reported, and kept in the raw row.
    let unknown = scalar_i64(
        &db_a,
        "SELECT count(*) FROM master_ingest_version WHERE unknown_keys->'cards' ? 'brandNewField'",
    )
    .await;
    assert_eq!(unknown, 1);
    assert_eq!(
        scalar_i64(
            &db_a,
            "SELECT count(*) FROM master_raw WHERE table_name = 'cards' AND raw ? 'brandNewField'"
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            &db_a,
            "SELECT count(*) FROM master_raw WHERE table_name = 'cards'"
        )
        .await,
        9
    );
    dbs.drop_all().await;
}

/// Full master directory (e.g. a region's git worktree):
/// `HARUKI_TEST_INGEST_MASTER_DIR=/path/to/master cargo test full_master -- --ignored`.
#[tokio::test]
#[ignore] // Requires PostgreSQL and a master directory
async fn full_master_directory_matches_the_old_path() {
    let Ok(dir) = std::env::var("HARUKI_TEST_INGEST_MASTER_DIR") else {
        eprintln!("HARUKI_TEST_INGEST_MASTER_DIR not set; skipped");
        return;
    };
    let mut dbs = Dbs::new().await;
    let (_, old) = dbs.create("ing_old").await;
    let (dsn_a, db_a) = dbs.create("ing_a").await;
    let (url, mock) = mock_registry("pg").await;
    let files = fixture_from(&dir);
    publish(&mock, &files, "1.0.0.1");
    // Typed rows only: raw rows would roughly double the scratch database.
    let mut a = target_cfg("full", &dsn_a);
    a.required_tables = TableSelection::Keyword("default".into());
    let ing = ingester(&url, 25, vec![a]).await;
    let started = std::time::Instant::now();
    let outcomes = ing.reconcile_region(JP).await;
    eprintln!("new path: {:?} in {:?}", outcomes, started.elapsed());
    let mapped = mapped_tables(&files);
    assert_eq!(written(outcome(&outcomes, "full")).0, mapped.len());
    let started = std::time::Instant::now();
    old_path_ingest(&old, &files).await;
    eprintln!("old path: {:?}", started.elapsed());
    for table in mapped.values() {
        assert_eq!(
            digest(&db_a, table).await,
            digest(&old, table).await,
            "{table}"
        );
    }
    let outcomes = ing.reconcile_region(JP).await;
    assert!(matches!(
        outcome(&outcomes, "full"),
        TargetOutcome::UpToDate { .. }
    ));
    dbs.drop_all().await;
}

/// A mapping change (raw turned on, a column dropped) re-ingests exactly the
/// affected tables although no master file changed.
#[tokio::test]
#[ignore] // Requires PostgreSQL (HARUKI_TEST_INGEST_DSN)
async fn mapping_change_reingests_only_affected_tables() {
    let mut dbs = Dbs::new().await;
    let (dsn, db) = dbs.create("ing_map").await;
    let (url, mock) = mock_registry("pg").await;
    let files = fixture();
    let mapped = mapped_tables(&files);
    publish(&mock, &files, "1.0.0.1");
    let cfg = target_cfg("main", &dsn);
    let outcomes = ingester(&url, 100, vec![cfg.clone()])
        .await
        .reconcile_region(JP)
        .await;
    assert_eq!(written(outcome(&outcomes, "main")).0, mapped.len());
    assert_eq!(scalar_i64(&db, "SELECT count(*) FROM master_raw").await, 0);

    // raw: true changes every table's mapping hash.
    let mut raw_cfg = cfg.clone();
    raw_cfg.raw = true;
    let ing = ingester(&url, 100, vec![raw_cfg]).await;
    let outcomes = ing.reconcile_region(JP).await;
    assert_eq!(written(outcome(&outcomes, "main")).0, mapped.len());
    assert!(scalar_i64(&db, "SELECT count(*) FROM master_raw").await > 0);
    assert_eq!(hits(&mock, &files["musics.json"]), 2);

    // A column dropped from one table: only that table is rewritten.
    db.execute_unprepared("ALTER TABLE musics DROP COLUMN lyricist")
        .await
        .unwrap();
    let outcomes = ing.reconcile_region(JP).await;
    assert_eq!(written(outcome(&outcomes, "main")), (1, 0, false));
    assert_eq!(hits(&mock, &files["musics.json"]), 3);
    assert_eq!(hits(&mock, &files["cards.json"]), 2);
    let outcomes = ing.reconcile_region(JP).await;
    assert!(matches!(
        outcome(&outcomes, "main"),
        TargetOutcome::UpToDate { .. }
    ));
    dbs.drop_all().await;
}

/// A file missing from one version keeps its rows; missing from the next
/// ingested version too, the rows are deleted. A file back in between
/// resets the count; a protected table is never cleared (the run fails).
#[tokio::test]
#[ignore] // Requires PostgreSQL (HARUKI_TEST_INGEST_DSN)
async fn removed_table_is_cleared_after_two_consecutive_missing_versions() {
    let mut dbs = Dbs::new().await;
    let (dsn, db) = dbs.create("ing_rm").await;
    let (url, mock) = mock_registry("pg").await;
    let files = fixture();
    publish(&mock, &files, "1.0.0.1");
    let mut cfg = target_cfg("main", &dsn);
    cfg.raw = true;
    let ing = ingester(&url, 100, vec![cfg]).await;
    ing.reconcile_region(JP).await;
    let tags = digest(&db, "musictags").await;
    assert!(tags.0 > 0);
    let missing = |db: DatabaseConnection| async move {
        scalar_i64(
            &db,
            "SELECT count(*) FROM master_ingest_state WHERE file = 'musicTags.json' \
             AND missing_since IS NOT NULL",
        )
        .await
    };

    let mut without_tags = files.clone();
    without_tags.remove("musicTags.json");
    publish(&mock, &without_tags, "1.0.0.2");
    let outcomes = ing.reconcile_region(JP).await;
    assert_eq!(written(outcome(&outcomes, "main")), (0, 0, false));
    assert_eq!(
        digest(&db, "musictags").await,
        tags,
        "first miss keeps rows"
    );
    assert_eq!(missing(db.clone()).await, 1);
    // Re-running the same version is not a second miss.
    ing.reconcile_region(JP).await;
    assert_eq!(digest(&db, "musictags").await, tags);

    // Back in between: the count restarts.
    publish(&mock, &files, "1.0.0.3");
    let outcomes = ing.reconcile_region(JP).await;
    assert_eq!(written(outcome(&outcomes, "main")), (0, 0, false));
    assert_eq!(missing(db.clone()).await, 0);
    publish(&mock, &without_tags, "1.0.0.4");
    ing.reconcile_region(JP).await;
    assert_eq!(digest(&db, "musictags").await, tags, "first miss again");

    // Second consecutive miss: rows (typed and raw) and state are gone.
    let v5 = with_rows(&without_tags, "events.json", |rows| {
        rows[0]["name"] = serde_json::json!("renamed");
    });
    publish(&mock, &v5, "1.0.0.5");
    let outcomes = ing.reconcile_region(JP).await;
    assert_eq!(written(outcome(&outcomes, "main")), (1, 1, false));
    assert_eq!(digest(&db, "musictags").await.0, 0);
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM master_raw WHERE table_name = 'musictags'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM master_ingest_state WHERE file = 'musicTags.json'"
        )
        .await,
        0
    );

    // A protected table disappearing fails the target and keeps everything.
    let mut without_cards = v5.clone();
    without_cards.remove("cards.json");
    publish(&mock, &without_cards, "1.0.0.6");
    let cards = digest(&db, "cards").await;
    let outcomes = ing.reconcile_region(JP).await;
    match outcome(&outcomes, "main") {
        TargetOutcome::Failed { error } => assert!(error.contains("protected"), "{error}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(digest(&db, "cards").await, cards);
    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT data_version, status FROM master_ingest_version",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "data_version").unwrap(),
        "1.0.0.5"
    );
    assert_eq!(row.try_get::<String>("", "status").unwrap(), "failed");
    dbs.drop_all().await;
}

/// `min_ratio` refuses a truncated table until allow-shrink names it for
/// that contentHash; the webhook needs the bearer token.
#[tokio::test]
#[ignore] // Requires PostgreSQL (HARUKI_TEST_INGEST_DSN)
async fn min_ratio_blocks_a_shrink_until_allowed_and_webhook_auth() {
    let mut dbs = Dbs::new().await;
    let (dsn, db) = dbs.create("ing_shrink").await;
    let (url, mock) = mock_registry("pg").await;
    let files = fixture();
    publish(&mock, &files, "1.0.0.1");
    let ing = ingester(&url, 100, vec![target_cfg("main", &dsn)]).await;
    ing.reconcile_region(JP).await;
    let musics = digest(&db, "musics").await;
    assert_eq!(musics.0, 40);

    let v2_files = with_rows(&files, "musics.json", |rows| rows.truncate(5));
    let v2 = publish(&mock, &v2_files, "1.0.0.2");
    let outcomes = ing.reconcile_region(JP).await;
    match outcome(&outcomes, "main") {
        TargetOutcome::Failed { error } => assert!(error.contains("min_ratio"), "{error}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(digest(&db, "musics").await, musics, "rolled back");

    let base = serve(super::http::router(ing.clone())).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({"contentHash": v2.content_hash, "tables": ["musics"]});
    let url_shrink = format!("{base}/v1/ingest/main/jp/allow-shrink");
    let resp = client.post(&url_shrink).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), 401);
    let resp = client
        .post(format!("{base}/v1/ingest/nope/jp/allow-shrink"))
        .bearer_auth("hook-token")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let resp = client
        .post(&url_shrink)
        .bearer_auth("hook-token")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // The POST triggered a background reconcile; run one here as well (the
    // advisory lock makes one of them skip as busy).
    let mut done = false;
    for _ in 0..100 {
        ing.reconcile_region(JP).await;
        if digest(&db, "musics").await.0 == 5 {
            done = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(done, "allow-shrink applied");
    assert_eq!(
        scalar_i64(
            &db,
            "SELECT count(*) FROM master_ingest_version WHERE status = 'ok' AND error LIKE '%allow-shrink%'"
        )
        .await,
        1
    );

    // Webhook: token required; a valid one triggers a reconcile.
    let hook = format!("{base}/internal/master-updated");
    let notice = serde_json::json!({"server": "jp", "dataVersion": "x"});
    let resp = client.post(&hook).json(&notice).send().await.unwrap();
    assert_eq!(resp.status(), 401);
    let resp = client
        .post(&hook)
        .bearer_auth("wrong")
        .json(&notice)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let resp = client
        .post(&hook)
        .bearer_auth("hook-token")
        .json(&notice)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let resp = client
        .post(&hook)
        .bearer_auth("hook-token")
        .json(&serde_json::json!({"server": "cn"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "no target ingests cn");
    let health: serde_json::Value = client
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(health["targets"]["main"]["jp"]["outcome"].is_string());
    dbs.drop_all().await;
}

/// A blob that 404s mid-run (the registry pruned that manifest) abandons the
/// run without recording anything and restarts from the new `current`.
#[tokio::test]
#[ignore] // Requires PostgreSQL (HARUKI_TEST_INGEST_DSN)
async fn pruned_blob_restarts_from_the_new_current() {
    let mut dbs = Dbs::new().await;
    let (dsn, db) = dbs.create("ing_gone").await;
    let (url, mock) = mock_registry("pg").await;
    let files = fixture();
    let v2_files = with_rows(&files, "events.json", |rows| rows.truncate(8));
    let v2 = manifest(&mock, &v2_files, "1.0.0.2");
    publish(&mock, &files, "1.0.0.1");
    {
        let mut m = mock.lock();
        m.gone.insert(sha(&files["events.json"]));
        m.on_gone = Some(v2.clone());
    }
    let ing = ingester(&url, 100, vec![target_cfg("main", &dsn)]).await;
    let outcomes = ing.reconcile_region(JP).await;
    match outcome(&outcomes, "main") {
        TargetOutcome::Ingested { content_hash, .. } => assert_eq!(content_hash, &v2.content_hash),
        other => panic!("{other:?}"),
    }
    assert_eq!(digest(&db, "events").await.0, 8);
    dbs.drop_all().await;
}

// ---------------------------------------------------------------- without a database

fn lazy_target(name: &str) -> Arc<Target> {
    let schema =
        MasterSchema::parse(&std::fs::read_to_string("schema_info.json").unwrap()).unwrap();
    let db = sea_orm::DatabaseConnection::default();
    Arc::new(Target::with_db(target_cfg(name, "postgres://unused/x"), schema, db).unwrap())
}

#[tokio::test]
async fn refuses_an_fs_registry() {
    let (url, _mock) = mock_registry("fs").await;
    let ing = Ingester::with_targets(ingest_cfg(&url, 25), vec![lazy_target("main")]);
    let err = ing.check_registry().await.unwrap_err().to_string();
    assert!(err.contains("blob_store: pg"), "{err}");
    let outcomes = ing.reconcile_region(JP).await;
    match outcome(&outcomes, "main") {
        TargetOutcome::Skipped { reason } => assert!(reason.contains("requires pg"), "{reason}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn webhook_is_off_without_a_token_and_validates_input() {
    let (url, _mock) = mock_registry("pg").await;
    let mut cfg = ingest_cfg(&url, 25);
    cfg.webhook_token.clear();
    let off = Arc::new(Ingester::with_targets(cfg, vec![lazy_target("main")]));
    let base = serve(super::http::router(off)).await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/internal/master-updated"))
        .bearer_auth("")
        .json(&serde_json::json!({"server": "jp"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let on = Arc::new(Ingester::with_targets(
        ingest_cfg(&url, 25),
        vec![lazy_target("main")],
    ));
    let base = serve(super::http::router(on)).await;
    for body in [serde_json::json!({"server": "xx"}), serde_json::json!({})] {
        let resp = client
            .post(format!("{base}/internal/master-updated"))
            .bearer_auth("hook-token")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "{body}");
    }
    let resp = client
        .post(format!("{base}/v1/ingest/main/jp/allow-shrink"))
        .bearer_auth("hook-token")
        .json(&serde_json::json!({"contentHash": "not-hex", "tables": ["cards"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let resp = client
        .post(format!("{base}/v1/ingest/main/jp/allow-shrink"))
        .bearer_auth("hook-token")
        .json(&serde_json::json!({"contentHash": "ab".repeat(32), "tables": ["noSuchTable"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn blob_reader_verifies_digest_and_reports_gone() {
    use std::io::Read;
    let (url, mock) = mock_registry("pg").await;
    let client = super::RegistryClient::new(&url);
    let good = b"[{\"id\":1}]".to_vec();
    let digest = sha(&good);
    mock.lock().blobs.insert(digest.clone(), good.clone());
    let mut reader = client
        .open_blob(JP, &digest, good.len() as u64)
        .await
        .unwrap();
    let body = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        reader.read_to_end(&mut out).map(|_| out)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(body, good);

    // Bytes that do not match the digest fail at the end of the stream.
    let wrong = "ab".repeat(32);
    mock.lock().blobs.insert(wrong.clone(), good.clone());
    let mut reader = client
        .open_blob(JP, &wrong, good.len() as u64)
        .await
        .unwrap();
    let read = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        reader.read_to_end(&mut out)
    })
    .await
    .unwrap();
    assert!(read.is_err());

    let missing = client.open_blob(JP, &"cd".repeat(32), 1).await;
    assert!(missing
        .err()
        .unwrap()
        .downcast_ref::<super::registry_client::Gone>()
        .is_some());
}

#[test]
fn plan_uses_the_matching_unique_index_and_hashes_the_mapping() {
    let target = lazy_target("main");
    let (cols, _) = target.schema.table("cards").unwrap();
    let mut shape = TableShape::default();
    for col in cols.keys() {
        shape.columns.insert(col.clone(), "text".into());
    }
    let plan_without_index = target.plan("cards", &shape).unwrap();
    assert!(plan_without_index.key.is_none());
    shape
        .unique_sets
        .push(["game_id", "server_region"].map(String::from).into());
    let plan = target.plan("cards", &shape).unwrap();
    assert_eq!(plan.key.as_deref(), Some(&["game_id".to_string()][..]));
    assert!(plan.has_region);
    assert!(!plan.cols.contains_key("server_region"));
    assert_ne!(plan.mapping_hash, plan_without_index.mapping_hash);
    assert_eq!(
        target.plan("cards", &shape).unwrap().mapping_hash,
        plan.mapping_hash
    );

    shape.columns.remove("prefix");
    let narrower = target.plan("cards", &shape).unwrap();
    assert!(!narrower.cols.contains_key("prefix"));
    assert_ne!(narrower.mapping_hash, plan.mapping_hash);

    let mut raw_cfg = target.cfg.clone();
    raw_cfg.raw = true;
    let schema =
        MasterSchema::parse(&std::fs::read_to_string("schema_info.json").unwrap()).unwrap();
    let raw_target =
        Target::with_db(raw_cfg, schema, sea_orm::DatabaseConnection::default()).unwrap();
    assert_ne!(
        raw_target.plan("cards", &shape).unwrap().mapping_hash,
        narrower.mapping_hash
    );

    assert_ne!(
        Target::skip_hash(Some("cards"), "not-selected"),
        Target::skip_hash(None, "unmapped")
    );
}

#[test]
fn target_table_lists_resolve_and_validate() {
    let schema =
        || MasterSchema::parse(&std::fs::read_to_string("schema_info.json").unwrap()).unwrap();
    let db = sea_orm::DatabaseConnection::default;
    let mut cfg = target_cfg("t", "postgres://unused/x");
    cfg.tables = TableSelection::List(vec!["cards".into(), "musicTags.json".into()]);
    cfg.required_tables = TableSelection::Keyword("default".into());
    let t = Target::with_db(cfg.clone(), schema(), db()).unwrap();
    assert!(t.is_selected("cards") && t.is_selected("musictags") && !t.is_selected("events"));
    // The default required list is narrowed to the allow-list.
    assert_eq!(t.required().iter().collect::<Vec<_>>(), vec!["cards"]);
    assert!(t.is_protected("events"), "prune protect list");
    assert!(!t.is_protected("musictags"));

    cfg.tables = TableSelection::List(vec!["noSuchTable".into()]);
    assert!(Target::with_db(cfg.clone(), schema(), db()).is_err());
    cfg.tables = TableSelection::Keyword("everything".into());
    assert!(Target::with_db(cfg.clone(), schema(), db()).is_err());
    cfg.tables = TableSelection::default();
    cfg.min_ratio = 1.5;
    assert!(Target::with_db(cfg, schema(), db()).is_err());
}

#[test]
fn config_validation() {
    let mut cfg = ingest_cfg("http://r", 25);
    assert!(super::validate(&cfg).is_err(), "no targets");
    cfg.targets = vec![
        target_cfg("a", "postgres://x/y"),
        target_cfg("a", "postgres://x/z"),
    ];
    assert!(super::validate(&cfg).is_err(), "duplicate name");
    cfg.targets[1].name = "bad name".into();
    assert!(super::validate(&cfg).is_err());
    cfg.targets[1].name = "b".into();
    super::validate(&cfg).unwrap();
    cfg.parse_concurrency = 0;
    assert!(super::validate(&cfg).is_err());
}

#[test]
fn typed_table_ddl_matches_the_ent_shape() {
    let schema =
        MasterSchema::parse(&std::fs::read_to_string("schema_info.json").unwrap()).unwrap();
    let ddl = typed_table_ddl(&schema, "areaitems").unwrap();
    assert!(ddl.contains("id bigserial PRIMARY KEY"));
    assert!(ddl.contains("\"server_region\" text NOT NULL"));
    assert!(ddl.contains("(\"game_id\", \"server_region\")"), "{ddl}");
    let ddl = typed_table_ddl(&schema, "ngwords").unwrap();
    assert!(!ddl.contains("UNIQUE"));
    assert!(typed_table_ddl(&schema, "nope").is_none());
}

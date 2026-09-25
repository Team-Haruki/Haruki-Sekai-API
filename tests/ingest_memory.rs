//! Peak heap of a registry ingest over large files, measured with a counting
//! global allocator (this test binary only). Requires PostgreSQL, like the
//! other ingest tests: set `HARUKI_TEST_INGEST_DSN` to a scratch database
//! whose user may create databases, then
//! `cargo test --test ingest_memory -- --ignored`.
//!
//! The files are shaped like the real worst cases: `cards.json` with few,
//! very large rows (JP `gachas.json` is 48 MB in 1011 rows) and
//! `resourceBoxes.json` with 150k small rows (TW/KR/CN `costume3ds.json` is
//! ~122k rows). Batches used to be bounded by row count only, so the
//! large-row file was parsed into one batch holding the whole file as a
//! `serde_json::Value` tree and copied twice more on its way into the INSERT.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use sea_orm::{ConnectionTrait, Database};
use serde_json::json;
use sha2::Digest as _;

use haruki_sekai_api::api::internal::{manifest_content_hash, MasterManifest, MasterManifestFile};
use haruki_sekai_api::config::{IngestConfig, IngestTargetConfig, ServerRegion, TableSelection};
use haruki_sekai_api::ingest::target::typed_table_ddl;
use haruki_sekai_api::ingest::{Ingester, Target, TargetOutcome};
use haruki_sekai_api::ingest_engine::MasterSchema;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = CURRENT.fetch_add(by, Relaxed) + by;
    PEAK.fetch_max(now, Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        CURRENT.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            if new_size >= layout.size() {
                grew(new_size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - new_size, Relaxed);
            }
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Peak heap above the level at the start of `f` (the files the mock
/// registry serves are allocated before and excluded).
async fn peak_during<T>(f: impl std::future::Future<Output = T>) -> (T, usize) {
    let base = CURRENT.load(Relaxed);
    PEAK.store(base, Relaxed);
    let out = f.await;
    (out, PEAK.load(Relaxed).saturating_sub(base))
}

/// Peak heap a region run may use on top of what it started with. Measured
/// at ~12 MiB for these files with byte-bounded batches; the row-count-only
/// batching needed ~1.8 GiB (all of `cards.json` in one batch).
const LIMIT: usize = 64 * 1024 * 1024;

const CARD_ROWS: usize = 1_000;
const PARAMS_PER_CARD: usize = 500;
const BOX_ROWS: usize = 150_000;

fn cards() -> Vec<u8> {
    let rows: Vec<serde_json::Value> = (1..=CARD_ROWS)
        .map(|id| {
            let params: Vec<serde_json::Value> = (0..PARAMS_PER_CARD)
                .map(|i| {
                    json!({"id": id * 100_000 + i, "cardId": id, "cardLevel": i % 60 + 1,
                           "cardParameterType": format!("param{}", i % 3 + 1), "power": 1000 + i})
                })
                .collect();
            json!({"id": id, "seq": id * 10, "characterId": id % 26 + 1,
                   "cardRarityType": "rarity_4", "attr": "cool", "supportUnit": "none",
                   "skillId": id, "cardSkillName": format!("skill {id}"), "prefix": "prefix",
                   "assetbundleName": format!("res{:03}_no{:03}", id % 26 + 1, id),
                   "releaseAt": 1_601_391_600_000i64, "cardParameters": params})
        })
        .collect();
    serde_json::to_vec(&rows).unwrap()
}

fn resource_boxes() -> Vec<u8> {
    let purposes = [
        "ad_reward",
        "mission_reward",
        "shop_item",
        "event_ranking_reward",
    ];
    let rows: Vec<serde_json::Value> = (0..BOX_ROWS)
        .map(|i| {
            let purpose = purposes[i % purposes.len()];
            let id = i / purposes.len() + 1;
            json!({"resourceBoxPurpose": purpose, "id": id, "resourceBoxType": "expand",
                   "description": format!("box {i}"),
                   "details": [{"resourceBoxPurpose": purpose, "resourceBoxId": id, "seq": 1,
                                "resourceType": "jewel", "resourceId": 1,
                                "resourceQuantity": i % 100 + 1}]})
        })
        .collect();
    serde_json::to_vec(&rows).unwrap()
}

type Blobs = Arc<HashMap<String, Bytes>>;

async fn mock_registry(manifest: MasterManifest, blobs: Blobs) -> String {
    let manifest = Arc::new(manifest);
    let router = Router::new()
        .route(
            "/health",
            get(|| async { axum::Json(json!({"blobStore": "pg"})) }),
        )
        .route(
            "/v1/master/{region}/current",
            get(move || {
                let manifest = manifest.clone();
                async move { axum::Json((*manifest).clone()) }
            }),
        )
        .route(
            "/v1/master/{region}/blob/{sha}",
            get(
                |State(blobs): State<Blobs>, Path((_r, sha)): Path<(String, String)>| async move {
                    match blobs.get(&sha) {
                        Some(b) => Response::new(Body::from(b.clone())),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
        )
        .with_state(blobs);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore] // Requires PostgreSQL (HARUKI_TEST_INGEST_DSN)
async fn large_files_ingest_in_bounded_memory() {
    let base = std::env::var("HARUKI_TEST_INGEST_DSN")
        .unwrap_or_else(|_| "postgres://haruki:sekai@localhost:5432/ingest_test".to_string());
    let admin = Database::connect(&base).await.unwrap();
    let name = format!("ing_mem_{}", uuid::Uuid::new_v4().simple());
    admin
        .execute_unprepared(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let (head, _) = base.rsplit_once('/').unwrap();
    let dsn = format!("{head}/{name}");
    {
        let db = Database::connect(&dsn).await.unwrap();
        let schema = MasterSchema::load("schema_info.json").await.unwrap();
        for table in ["cards", "resourceboxes"] {
            db.execute_unprepared(&typed_table_ddl(&schema, table).unwrap())
                .await
                .unwrap();
        }
    }

    let mut blobs = HashMap::new();
    let mut entries = Vec::new();
    for (file, bytes) in [
        ("cards.json", cards()),
        ("resourceBoxes.json", resource_boxes()),
    ] {
        let sha = hex::encode(sha2::Sha256::digest(&bytes));
        eprintln!("{file}: {} bytes", bytes.len());
        entries.push(MasterManifestFile {
            name: file.into(),
            size: bytes.len() as u64,
            sha256: sha.clone(),
        });
        blobs.insert(sha, Bytes::from(bytes));
    }
    let manifest = MasterManifest {
        server: "jp".into(),
        app_version: "6.0.0".into(),
        app_hash: "h".into(),
        data_version: "6.0.0.10".into(),
        asset_version: "6.0.0.10".into(),
        asset_hash: "a".into(),
        cdn_version: 0,
        generated_at: "2026-09-25T00:00:00Z".into(),
        content_hash: manifest_content_hash(&entries),
        git_commit: None,
        files: entries,
    };
    let url = mock_registry(manifest, Arc::new(blobs)).await;

    let mut target: IngestTargetConfig =
        serde_yaml::from_str(&format!("name: mem\ndsn: \"{dsn}\"\n")).unwrap();
    target.regions = vec![ServerRegion::Jp];
    target.required_tables = TableSelection::List(vec![]);
    let mut cfg: IngestConfig =
        serde_yaml::from_str(&format!("registry_url: \"{url}\"\n")).unwrap();
    // Staged per file, as a first full ingest is.
    cfg.stage_tables = 1;
    cfg.targets = vec![target.clone()];
    let ingester =
        Ingester::with_targets(cfg, vec![Arc::new(Target::open(target, 5).await.unwrap())]);

    // First ingest into empty tables, then the production case: the rows
    // exist (written by the old path) but no ingest state does, so every
    // file is upserted against the existing rows.
    for pass in ["empty tables", "existing rows, no state"] {
        let (outcomes, peak) = peak_during(ingester.reconcile_region(ServerRegion::Jp)).await;
        eprintln!("{pass}: peak heap {:.1} MiB", peak as f64 / 1048576.0);
        assert!(
            matches!(outcomes[0].1, TargetOutcome::Ingested { written: 2, .. }),
            "{pass}: {outcomes:?}"
        );
        assert!(
            peak < LIMIT,
            "{pass}: peak heap {peak} bytes is over {LIMIT}"
        );
        let db = Database::connect(&dsn).await.unwrap();
        db.execute_unprepared("DELETE FROM master_ingest_state; DELETE FROM master_ingest_version")
            .await
            .unwrap();
    }

    let _ = admin
        .execute_unprepared(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
        .await;
}

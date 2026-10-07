//! Per-stage CPU / allocation profile of the game-API proxy path on a real
//! response body (JSON fixture -> msgpack -> AES, then the production decode
//! chain), plus the response-side costs (cache entry, JSON serialization,
//! gzip/zstd at the levels tower-http uses).
//!
//! Usage:
//!   STAGES_FIXTURE=/path/to/ranking.json STAGES_PATH='/user/{userId}/event/181/ranking?rankingViewType=top100' \
//!   STAGES_BUNDLE=Data/structures/nuverse_schema_bundle.json cargo run --release --bin bench_stages
//!
//! Env: STAGES_ITERS (default 200), STAGES_NUVERSE=0 to skip the restore stage.
//! The fixture is a captured response body (e.g. a cached ranking entry);
//! it is never committed.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use haruki_sekai_api::client::nuverse_schema::NuverseSchemaStore;
use haruki_sekai_api::crypto::SekaiCryptor;

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        INNER.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        INNER.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        if new_size > layout.size() {
            BYTES.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            let live = LIVE.fetch_add(new_size - layout.size(), Ordering::Relaxed) + new_size
                - layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        } else {
            LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
        }
        INNER.realloc(ptr, layout, new_size)
    }
}

/// Same allocator the binaries use, wrapped by the counter.
static INNER: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: Counting = Counting;

struct Measure {
    name: &'static str,
    samples: Vec<f64>,
    allocs: usize,
    bytes: usize,
    peak: usize,
}

fn measure<R>(name: &'static str, iters: usize, mut f: impl FnMut() -> R) -> Measure {
    // warmup
    for _ in 0..3 {
        std::hint::black_box(f());
    }
    let mut samples = Vec::with_capacity(iters);
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let b0 = BYTES.load(Ordering::Relaxed);
    let live0 = LIVE.load(Ordering::Relaxed);
    PEAK.store(live0, Ordering::Relaxed);
    for _ in 0..iters {
        let t = Instant::now();
        let r = f();
        samples.push(t.elapsed().as_secs_f64() * 1e6);
        drop(std::hint::black_box(r));
    }
    let allocs = (ALLOCS.load(Ordering::Relaxed) - a0) / iters;
    let bytes = (BYTES.load(Ordering::Relaxed) - b0) / iters;
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(live0);
    Measure {
        name,
        samples,
        allocs,
        bytes,
        peak,
    }
}

fn report(m: &Measure) {
    let mut s = m.samples.clone();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let mean = s.iter().sum::<f64>() / n as f64;
    println!(
        "{:<44} p50 {:>9.1} us  mean {:>9.1} us  p95 {:>9.1} us | {:>6} allocs {:>9} B alloc'd  peak {:>9} B",
        m.name,
        s[n / 2],
        mean,
        s[(n as f64 * 0.95) as usize],
        m.allocs,
        m.bytes,
        m.peak
    );
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn main() {
    let fixture = env("STAGES_FIXTURE").expect("STAGES_FIXTURE=<json file>");
    let path = env("STAGES_PATH")
        .unwrap_or_else(|| "/user/{userId}/event/181/ranking?rankingViewType=top100".into());
    let iters: usize = env("STAGES_ITERS")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let json_text = std::fs::read_to_string(&fixture).expect("read fixture");
    let value: serde_json::Value = serde_json::from_str(&json_text).expect("fixture json");

    // Build the wire body the game server would send: msgpack of the response,
    // AES-128-CBC + PKCS7. (Nuverse would send compact arrays for userCard; we
    // restore from objects, which exercises the same tree walk.)
    let msgpack = rmp_serde::to_vec_named(&value).expect("msgpack");
    let cryptor = SekaiCryptor::from_hex(
        "00112233445566778899aabbccddeeff",
        "ffeeddccbbaa99887766554433221100",
    )
    .unwrap();
    let body = cryptor.pack_bytes(&msgpack).unwrap();
    let body_bytes = bytes::Bytes::from(body.clone());
    let store = match (env("STAGES_BUNDLE"), env("STAGES_NUVERSE").as_deref()) {
        (Some(p), Some("0")) => {
            let _ = p;
            None
        }
        (None, _) => None,
        (Some(p), _) => Some(NuverseSchemaStore::from_slice(&std::fs::read(p).unwrap()).unwrap()),
    };
    let decoded = cryptor.unpack_value(&body).unwrap();
    let json_out = sonic_rs::to_string(&decoded).unwrap();
    println!(
        "fixture {} : json {} B, msgpack {} B, encrypted {} B, restore={}\n",
        fixture,
        json_text.len(),
        msgpack.len(),
        body.len(),
        store.is_some()
    );

    let mut rows = Vec::new();
    rows.push(measure("decrypt_msgpack (AES-CBC + copy)", iters, || {
        cryptor.decrypt_msgpack(&body).unwrap()
    }));
    rows.push(measure(
        "msgpack -> serde_json::Value (rmpv ref)",
        iters,
        || haruki_sekai_api::crypto::decode_msgpack_value(&msgpack).unwrap(),
    ));
    rows.push(measure("unpack_value (decrypt + decode)", iters, || {
        cryptor.unpack_value(&body).unwrap()
    }));
    rows.push(measure("unpack_value from Bytes (as prod)", iters, || {
        cryptor.unpack_value(&body_bytes).unwrap()
    }));
    if let Some(store) = &store {
        rows.push(measure("nuverse restore_api_json", iters, || {
            store.restore_api_json(&path, decoded.clone()).unwrap()
        }));
        rows.push(measure("  (Value clone baseline)", iters, || {
            decoded.clone()
        }));
    }
    rows.push(measure("sonic_rs::to_string(Value)", iters, || {
        sonic_rs::to_string(&decoded).unwrap()
    }));
    rows.push(measure("serde_json::to_string(Value)", iters, || {
        serde_json::to_string(&decoded).unwrap()
    }));
    rows.push(measure("serde_json::to_vec(Value)", iters, || {
        serde_json::to_vec(&decoded).unwrap()
    }));
    rows.push(measure(
        "cache entry encode (format ts|json)",
        iters,
        || format!("{}|{}", 1_700_000_000_000u64, json_out),
    ));
    let entry = format!("{}|{}", 1_700_000_000_000u64, json_out);
    rows.push(measure("cache hit: split + to_string copy", iters, || {
        let (_, j) = entry.split_once('|').unwrap();
        j.to_string()
    }));
    rows.push(measure("Arc<str> from String", iters, || {
        std::sync::Arc::<str>::from(json_out.clone())
    }));
    rows.push(measure("gzip level 6 (tower-http default)", iters, || {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(
            Vec::with_capacity(json_out.len() / 4),
            flate2::Compression::new(6),
        );
        e.write_all(json_out.as_bytes()).unwrap();
        e.finish().unwrap()
    }));
    for (name, level) in [
        ("gzip level 4", 4u32),
        ("gzip level 3", 3),
        ("gzip level 2", 2),
        ("gzip level 1", 1),
    ] {
        rows.push(measure(name, iters, || {
            use std::io::Write;
            let mut e = flate2::write::GzEncoder::new(
                Vec::with_capacity(json_out.len() / 4),
                flate2::Compression::new(level),
            );
            e.write_all(json_out.as_bytes()).unwrap();
            e.finish().unwrap()
        }));
    }
    rows.push(measure("zstd level 3 (default)", iters, || {
        zstd::bulk::compress(json_out.as_bytes(), 3).unwrap()
    }));
    rows.push(measure("zstd level 1", iters, || {
        zstd::bulk::compress(json_out.as_bytes(), 1).unwrap()
    }));
    let g6 = {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        e.write_all(json_out.as_bytes()).unwrap();
        e.finish().unwrap()
    };
    let g1 = {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(1));
        e.write_all(json_out.as_bytes()).unwrap();
        e.finish().unwrap()
    };
    let z3 = zstd::bulk::compress(json_out.as_bytes(), 3).unwrap();
    let z1 = zstd::bulk::compress(json_out.as_bytes(), 1).unwrap();
    let gz = |level: u32| {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(level));
        e.write_all(json_out.as_bytes()).unwrap();
        e.finish().unwrap().len()
    };
    println!(
        "sizes: json {} gz6 {} gz4 {} gz3 {} gz2 {} gz1 {} zstd3 {} zstd1 {}\n",
        json_out.len(),
        g6.len(),
        gz(4),
        gz(3),
        gz(2),
        g1.len(),
        z3.len(),
        z1.len()
    );

    // JWT auth check, as the middleware does per request.
    let secret = "bench-secret";
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &serde_json::json!({"uid":"user","credential":"abcdefabcdefabcdef"}),
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();
    rows.push(measure("jwt HS256 decode (auth middleware)", iters, || {
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        v.required_spec_claims.clear();
        v.validate_exp = false;
        jsonwebtoken::decode::<serde_json::Value>(
            &token,
            &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
            &v,
        )
        .unwrap()
    }));

    // Full axum response path through CompressionLayer (what the server does
    // for a cached hit): handler returns the JSON string, gzip negotiated.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let json_for_router = std::sync::Arc::<str>::from(json_out.as_str());
    let mk_app = |compress: bool| {
        use axum::{routing::get, Router};
        let j = json_for_router.clone();
        let r = Router::new().route(
            "/x",
            get(move || {
                let j = j.clone();
                async move { ([("content-type", "application/json")], j.to_string()) }
            }),
        );
        if compress {
            r.layer(tower_http::compression::CompressionLayer::new())
        } else {
            r
        }
    };
    for (name, compress, enc) in [
        ("axum response, no compression layer", false, ""),
        ("axum + CompressionLayer, identity", true, "identity"),
        ("axum + CompressionLayer, gzip", true, "gzip"),
        ("axum + CompressionLayer, zstd", true, "zstd"),
    ] {
        let app = mk_app(compress);
        rows.push(measure(name, iters, || {
            use tower::ServiceExt;
            let app = app.clone();
            rt.block_on(async move {
                let mut req = axum::http::Request::builder().uri("/x");
                if !enc.is_empty() {
                    req = req.header("accept-encoding", enc);
                }
                let resp = app
                    .oneshot(req.body(axum::body::Body::empty()).unwrap())
                    .await
                    .unwrap();
                axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .len()
            })
        }));
    }

    for m in &rows {
        report(m);
    }
}

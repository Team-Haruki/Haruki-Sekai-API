//! Profile API per-stage latency benchmark (Nuverse / CN by default).
//!
//! Breaks down a `/user/{userId}/{targetUserId}/profile` proxy call into the
//! same stages the production path runs and archives the timings:
//!   network (TTFB) -> body download -> AES decrypt -> msgpack decode ->
//!   IndexMap -> serde_json::Value deep clone -> Nuverse schema restore ->
//!   final JSON serialization.
//!
//! It reuses the real `SekaiClient` request path and crypto, so the numbers
//! reflect production behavior rather than a reimplementation.
//!
//! The target user id is read from `BENCH_PROFILE_UID` so it never appears in
//! committed source. Results are written under `Data/benchmarks/` (gitignored).
//!
//! Usage:
//!   BENCH_PROFILE_UID=<uid> cargo run --release --bin bench_profile
//!
//! Optional env:
//!   BENCH_REGION      region key (default: cn)
//!   BENCH_CPU_ITERS   CPU-pipeline samples on a captured body (default: 300)
//!   BENCH_E2E_ITERS   real end-to-end calls incl. network (default: 15)
//!   BENCH_WARMUP      warmup iterations skipped from stats (default: 3)
//!   BENCH_E2E_DELAYMS delay between e2e calls, ms (default: 150)
//!   BENCH_PROXY       override config proxy ("" forces direct connection)
//!   BENCH_APP_VERSION / BENCH_APP_HASH / BENCH_DATA_VERSION / BENCH_ASSET_VERSION
//!                     override the corresponding X-* request header after init
//!                     (use when the local version file is stale -> HTTP 426)

use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};
use haruki_sekai_api::client::SekaiClient;
use haruki_sekai_api::config::{Config, ServerRegion};
use serde::Serialize;
use serde_json::Value as JsonValue;

#[derive(Default, Clone, Serialize)]
struct Stat {
    n: usize,
    min_us: f64,
    p50_us: f64,
    mean_us: f64,
    p95_us: f64,
    max_us: f64,
}

#[derive(Clone, Serialize)]
struct StageRow {
    stage: String,
    stat: Stat,
}

#[derive(Serialize)]
struct Report {
    timestamp: String,
    region: String,
    target_uid_redacted: String,
    upstream_status: u16,
    encrypted_body_bytes: usize,
    restored_json_bytes: usize,
    user_honors_count: usize,
    user_profile_honors_count: usize,
    raw_user_honors_first_is_array: Option<bool>,
    restored_user_honors_first_is_object: Option<bool>,
    cpu_iters: usize,
    e2e_iters: usize,
    warmup: usize,
    e2e_errors: usize,
    cpu_stages: Vec<StageRow>,
    e2e_stages: Vec<StageRow>,
}

fn summarize(mut samples: Vec<f64>) -> Stat {
    if samples.is_empty() {
        return Stat::default();
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    let sum: f64 = samples.iter().sum();
    let pct = |p: f64| -> f64 {
        let idx = ((p / 100.0) * ((n - 1) as f64)).round() as usize;
        samples[idx]
    };
    Stat {
        n,
        min_us: samples[0],
        p50_us: pct(50.0),
        mean_us: sum / n as f64,
        p95_us: pct(95.0),
        max_us: samples[n - 1],
    }
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000_000.0
}

fn fmt_dur(value_us: f64) -> String {
    if value_us >= 1000.0 {
        format!("{:.3} ms", value_us / 1000.0)
    } else {
        format!("{value_us:.1} us")
    }
}

fn redact_uid(uid: &str) -> String {
    if uid.chars().count() <= 4 {
        "****".to_string()
    } else {
        format!("***{}", &uid[uid.len() - 4..])
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn row(stage: &str, stat: &Stat) -> StageRow {
    StageRow {
        stage: stage.to_string(),
        stat: stat.clone(),
    }
}

fn render_table(title: &str, rows: &[StageRow]) -> String {
    let mut out = format!("## {title}\n\n");
    out.push_str("| stage | n | min | p50 | mean | p95 | max |\n");
    out.push_str("|---|---:|---:|---:|---:|---:|---:|\n");
    for r in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            r.stage,
            r.stat.n,
            fmt_dur(r.stat.min_us),
            fmt_dur(r.stat.p50_us),
            fmt_dur(r.stat.mean_us),
            fmt_dur(r.stat.p95_us),
            fmt_dur(r.stat.max_us),
        ));
    }
    out.push('\n');
    out
}

/// Override version/hash request headers from env, applied after `init()` so
/// the operator can feed CURRENT values without editing the version file.
fn apply_header_overrides(client: &SekaiClient) {
    let overrides = [
        ("X-App-Version", "BENCH_APP_VERSION"),
        ("X-App-Hash", "BENCH_APP_HASH"),
        ("X-Data-Version", "BENCH_DATA_VERSION"),
        ("X-Asset-Version", "BENCH_ASSET_VERSION"),
    ];
    let mut headers = client.headers.lock();
    for (header, env_key) in overrides {
        if let Ok(value) = std::env::var(env_key) {
            let shown = if header.contains("Hash") && value.len() > 8 {
                format!("{}...", &value[..8])
            } else {
                value.clone()
            };
            println!("header override: {header} = '{shown}'");
            headers.insert(header.to_string(), value);
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let target_uid = std::env::var("BENCH_PROFILE_UID")
        .context("BENCH_PROFILE_UID env var is required (the target user id to query)")?;
    if target_uid.trim().is_empty() {
        return Err(anyhow!("BENCH_PROFILE_UID must not be empty"));
    }
    let region_key = std::env::var("BENCH_REGION").unwrap_or_else(|_| "cn".to_string());
    let region: ServerRegion = region_key
        .parse()
        .map_err(|_| anyhow!("invalid BENCH_REGION '{region_key}'"))?;
    let cpu_iters = env_usize("BENCH_CPU_ITERS", 300);
    let e2e_iters = env_usize("BENCH_E2E_ITERS", 15);
    let warmup = env_usize("BENCH_WARMUP", 3);
    let e2e_delay = Duration::from_millis(env_usize("BENCH_E2E_DELAYMS", 150) as u64);

    let redacted = redact_uid(&target_uid);
    println!("== Profile bench :: region={region_key} target={redacted} ==");

    // --- Build the real client, reusing production construction + init ---
    let config = Config::load().context("failed to load config")?;
    let server_config = config
        .servers
        .get(&region)
        .with_context(|| format!("region '{region_key}' not present in config.servers"))?
        .clone();

    let proxy = match std::env::var("BENCH_PROXY") {
        Ok(p) if p.is_empty() => None,
        Ok(p) => Some(p),
        Err(_) => (!config.proxy.is_empty()).then(|| config.proxy.clone()),
    };
    let jp_cookie_url =
        (!config.jp_sekai_cookie_url.is_empty()).then(|| config.jp_sekai_cookie_url.clone());
    println!(
        "proxy={} api_url={}",
        proxy.as_deref().unwrap_or("(direct)"),
        server_config.api_url
    );

    let http_client =
        SekaiClient::build_http_client(proxy.as_deref()).context("build http client")?;
    let nuverse_store =
        if region.is_cp_server() || server_config.nuverse_schema_bundle_path.is_empty() {
            None
        } else {
            Some(std::sync::Arc::new(
                SekaiClient::load_nuverse_schema_store(&server_config.nuverse_schema_bundle_path)
                    .context("load nuverse schema bundle")?,
            ))
        };
    let client = SekaiClient::new(
        region,
        server_config,
        proxy,
        jp_cookie_url,
        http_client,
        nuverse_store,
    )
    .await
    .context("SekaiClient::new failed")?;
    println!("logging in (init)...");
    client.init().await.context("client.init() failed")?;
    if client.get_session().is_none() {
        return Err(anyhow!(
            "no usable session after init (login failed?). Check account dir / proxy / version file."
        ));
    }

    // Optional header overrides so the operator can supply CURRENT version/hash
    // values (the cluster keeps these fresh via the apphash/master updaters; a
    // stale version file yields HTTP 426 and a non-representative payload).
    apply_header_overrides(&client);

    // Mirror production's get_game_api 426 handler: backfill data/asset version
    // from a fresh login response (init() logs in but does NOT apply these).
    // An explicit env override (BENCH_DATA_VERSION / BENCH_ASSET_VERSION) wins.
    let login_session = client.get_session().context("no session")?;
    match client.login(&login_session).await {
        Ok(login) => {
            let uid = login.user_registration.as_ref().map_or_else(
                || login_session.user_id().to_string(),
                |r| r.user_id.clone(),
            );
            println!(
                "login ok: bot_uid={} cdnVersion={} dataVersion={} assetVersion={}",
                redact_uid(&uid),
                login.cdn_version,
                login.data_version,
                login.asset_version
            );
            let mut headers = client.headers.lock();
            if std::env::var("BENCH_DATA_VERSION").is_err() && !login.data_version.is_empty() {
                headers.insert("X-Data-Version".to_string(), login.data_version);
            }
            if std::env::var("BENCH_ASSET_VERSION").is_err() && !login.asset_version.is_empty() {
                headers.insert("X-Asset-Version".to_string(), login.asset_version);
            }
        }
        Err(e) => eprintln!("warning: explicit login for version backfill failed: {e}"),
    }

    // The path keeps the literal {userId} placeholder, exactly like the handler:
    // call_api_with_timeout substitutes it for the URL, restore_api matches the pattern.
    let path = format!("/user/{{userId}}/{target_uid}/profile");

    // --- One real call to capture a body + diagnostics ---
    println!("capturing one profile response...");
    let session = client.get_session().context("no session")?;
    let resp = client
        .get(&session, &path, None)
        .await
        .context("initial profile GET failed")?;
    let status = resp.status().as_u16();
    let body = resp.bytes().await.context("read body failed")?;
    let encrypted_len = body.len();
    println!("upstream status={status}, encrypted body={encrypted_len} bytes");
    if status != 200 {
        eprintln!(
            "WARNING: upstream returned {status} (not 200). The body is an error/upgrade stub, \
so the CPU-stage numbers below are NOT representative of a real profile payload. \
Supply current headers via BENCH_APP_VERSION / BENCH_APP_HASH / BENCH_DATA_VERSION / \
BENCH_ASSET_VERSION, or run inside the cluster where the updaters keep them fresh."
        );
    }

    // Inspect raw (pre-restore) vs restored shape to show exactly what restore does.
    let raw_imap = client
        .cryptor
        .unpack_ordered(&body)
        .context("unpack_ordered failed (non-200 / non-octet-stream body?)")?;
    let raw_value = serde_json::to_value(&raw_imap)?;
    if status != 200 {
        eprintln!(
            "error body: {}",
            serde_json::to_string(&raw_value).unwrap_or_default()
        );
    }
    // Prove the NEW path (unpack_value) yields byte-identical JSON to the OLD
    // path (unpack_ordered -> to_value) on the real payload.
    let value_new_once = client.cryptor.unpack_value(&body)?;
    let decode_paths_match = value_new_once == raw_value;
    println!(
        "decode equivalence (unpack_value == unpack_ordered + to_value): {decode_paths_match}"
    );

    let raw_user_honors_first_is_array = raw_value
        .get("userHonors")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .map(JsonValue::is_array);
    let restored = client.restore_nuverse_api_response(&path, raw_value)?;
    let restored_user_honors_first_is_object = restored
        .get("userHonors")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .map(JsonValue::is_object);
    let user_honors_count = restored
        .get("userHonors")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    let user_profile_honors_count = restored
        .get("userProfileHonors")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    let restored_json_len = sonic_rs::to_string(&restored)?.len();

    println!(
        "userHonors={user_honors_count} userProfileHonors={user_profile_honors_count} restored_json={restored_json_len} bytes"
    );
    println!(
        "diag: raw_userHonors_first_is_array={raw_user_honors_first_is_array:?} restored_first_is_object={restored_user_honors_first_is_object:?}"
    );

    // --- CPU pipeline microbench on the captured body ---
    // Measures both the NEW production path (handle_response_value -> unpack_value,
    // no IndexMap / no to_value deep clone) and the OLD path (unpack_ordered +
    // to_value) so the optimization shows up as a before/after in a single run.
    println!("\nrunning CPU pipeline x{cpu_iters} (warmup {warmup})...");
    let mut s_decrypt = Vec::new();
    let mut s_unpack_value = Vec::new();
    let mut s_unpack_ordered = Vec::new();
    let mut s_to_value = Vec::new();
    let mut s_restore = Vec::new();
    let mut s_serialize = Vec::new();
    let mut s_cpu_new = Vec::new();
    let mut s_cpu_old = Vec::new();
    for i in 0..cpu_iters {
        let record = i >= warmup;

        let t = Instant::now();
        let _msgpack = client.cryptor.decrypt_msgpack(&body)?;
        let d_decrypt = t.elapsed();

        // NEW path: decrypt + decode -> serde_json::Value directly (unpack_value)
        let t = Instant::now();
        let value_new = client.cryptor.unpack_value(&body)?;
        let d_unpack_value = t.elapsed();

        // OLD path: decrypt + decode -> IndexMap, then IndexMap -> Value deep clone
        let t = Instant::now();
        let imap = client.cryptor.unpack_ordered(&body)?;
        let d_unpack_ordered = t.elapsed();
        let t = Instant::now();
        let _value_old = serde_json::to_value(&imap)?;
        let d_to_value = t.elapsed();

        // Nuverse schema (Avro-bundle) restore
        let t = Instant::now();
        let value = client.restore_nuverse_api_response(&path, value_new)?;
        let d_restore = t.elapsed();

        // final serialization (ApiResponse::into_response)
        let t = Instant::now();
        let _json = sonic_rs::to_string(&value)?;
        let d_serialize = t.elapsed();

        if record {
            s_decrypt.push(us(d_decrypt));
            s_unpack_value.push(us(d_unpack_value));
            s_unpack_ordered.push(us(d_unpack_ordered));
            s_to_value.push(us(d_to_value));
            s_restore.push(us(d_restore));
            s_serialize.push(us(d_serialize));
            s_cpu_new.push(us(d_unpack_value + d_restore + d_serialize));
            s_cpu_old.push(us(d_unpack_ordered + d_to_value + d_restore + d_serialize));
        }
    }

    let st_decrypt = summarize(s_decrypt);
    let st_unpack_value = summarize(s_unpack_value);
    let st_unpack_ordered = summarize(s_unpack_ordered);
    let st_to_value = summarize(s_to_value);
    let st_restore = summarize(s_restore);
    let st_serialize = summarize(s_serialize);
    let st_cpu_new = summarize(s_cpu_new);
    let st_cpu_old = summarize(s_cpu_old);

    // --- End-to-end real calls (network included) ---
    println!("running end-to-end x{e2e_iters} (warmup {warmup})...");
    let mut s_net = Vec::new();
    let mut s_body = Vec::new();
    let mut s_cpu = Vec::new();
    let mut s_e2e_total = Vec::new();
    let mut e2e_errors = 0usize;
    for i in 0..e2e_iters {
        if i > 0 {
            tokio::time::sleep(e2e_delay).await;
        }
        let record = i >= warmup;
        let Some(session) = client.get_session() else {
            break;
        };
        let t0 = Instant::now();
        let resp = match client.get(&session, &path, None).await {
            Ok(r) => r,
            Err(e) => {
                e2e_errors += 1;
                eprintln!("e2e call {i} failed (network): {e}");
                continue;
            }
        };
        let d_net = t0.elapsed();
        let t1 = Instant::now();
        let body = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                e2e_errors += 1;
                eprintln!("e2e call {i} body read failed: {e}");
                continue;
            }
        };
        let d_body = t1.elapsed();

        let t2 = Instant::now();
        let value = client.cryptor.unpack_value(&body)?;
        let value = client.restore_nuverse_api_response(&path, value)?;
        let _json = sonic_rs::to_string(&value)?;
        let d_cpu = t2.elapsed();
        let d_total = t0.elapsed();

        if record {
            s_net.push(us(d_net));
            s_body.push(us(d_body));
            s_cpu.push(us(d_cpu));
            s_e2e_total.push(us(d_total));
        }
    }
    let st_net = summarize(s_net);
    let st_body = summarize(s_body);
    let st_cpu = summarize(s_cpu);
    let st_e2e_total = summarize(s_e2e_total);

    // --- Assemble report ---
    let cpu_stages = vec![
        row("decrypt (AES-128-CBC + unpad)", &st_decrypt),
        row(
            "[NEW] decrypt+decode -> Value (unpack_value)",
            &st_unpack_value,
        ),
        row(
            "[OLD] decrypt+decode -> IndexMap (unpack_ordered)",
            &st_unpack_ordered,
        ),
        row(
            "[OLD] IndexMap -> Value deep clone (to_value)",
            &st_to_value,
        ),
        row("schema/Avro restore (restore_api_json)", &st_restore),
        row("serialize (sonic_rs::to_string)", &st_serialize),
        row(
            "[NEW] CPU total (unpack_value + restore + serialize)",
            &st_cpu_new,
        ),
        row(
            "[OLD] CPU total (unpack_ordered + to_value + ...)",
            &st_cpu_old,
        ),
    ];
    let e2e_stages = vec![
        row("network TTFB (get -> response headers)", &st_net),
        row("body download (resp.bytes)", &st_body),
        row("CPU pipeline (full, once)", &st_cpu),
        row("end-to-end total", &st_e2e_total),
    ];

    let ts = chrono::Local::now();
    let mut md = String::new();
    md.push_str("# Profile API benchmark\n\n");
    md.push_str(&format!("- timestamp: {}\n", ts.to_rfc3339()));
    md.push_str(&format!("- region: `{region_key}`\n"));
    md.push_str(&format!("- target uid: `{redacted}` (redacted)\n"));
    md.push_str(&format!("- upstream status: `{status}`\n"));
    md.push_str(&format!("- encrypted body: {encrypted_len} bytes\n"));
    md.push_str(&format!("- restored JSON: {restored_json_len} bytes\n"));
    md.push_str(&format!(
        "- userHonors: {user_honors_count}, userProfileHonors: {user_profile_honors_count}\n"
    ));
    md.push_str(&format!(
        "- diagnostic: raw userHonors[0] is array = `{raw_user_honors_first_is_array:?}`, restored[0] is object = `{restored_user_honors_first_is_object:?}`\n"
    ));
    md.push_str(&format!(
        "- iterations: cpu={cpu_iters} (warmup {warmup}), e2e={e2e_iters} (errors {e2e_errors})\n\n"
    ));
    md.push_str(&render_table("CPU pipeline (captured body)", &cpu_stages));
    md.push_str(&render_table("End-to-end (real network)", &e2e_stages));

    print!("\n{md}");

    let report = Report {
        timestamp: ts.to_rfc3339(),
        region: region_key.clone(),
        target_uid_redacted: redacted,
        upstream_status: status,
        encrypted_body_bytes: encrypted_len,
        restored_json_bytes: restored_json_len,
        user_honors_count,
        user_profile_honors_count,
        raw_user_honors_first_is_array,
        restored_user_honors_first_is_object,
        cpu_iters,
        e2e_iters,
        warmup,
        e2e_errors,
        cpu_stages,
        e2e_stages,
    };

    // Archive under Data/benchmarks/ (gitignored). Never write the raw uid.
    let dir = std::path::Path::new("Data/benchmarks");
    std::fs::create_dir_all(dir).context("create Data/benchmarks")?;
    let stamp = ts.format("%Y%m%d-%H%M%S");
    let md_path = dir.join(format!("profile-{region_key}-{stamp}.md"));
    let json_path = dir.join(format!("profile-{region_key}-{stamp}.json"));
    std::fs::write(&md_path, &md).context("write md archive")?;
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?).context("write json")?;

    println!(
        "archived:\n  {}\n  {}",
        md_path.display(),
        json_path.display()
    );
    Ok(())
}

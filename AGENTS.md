# AGENTS.md — AI Agent Guidelines for Haruki Sekai API

## Project Summary

Haruki Sekai API is a Rust service that proxies encrypted API calls to regional game servers of "Project Sekai: Colorful Stage". It manages multi-region game accounts, auto-updates master data, and serves a REST API for HarukiBot.

## Repository Structure

```
src/
  main.rs                  – App bootstrap (config, server, graceful shutdown)
  logging.rs               – Tracing setup, custom console formatter (main-only module)
  lib.rs                   – Public modules, AppState struct, RequestCoalescer (single-flight)
  config.rs                – YAML config structs with serde defaults
  error.rs                 – AppError enum (thiserror), HTTP status mapping, IntoResponse
  utils.rs                 – retry_async(), CachedResource<T>
  ingest_engine.rs         – Bulk JSON→DB ingestion using schema_info.json (the in-sync path);
                             `MasterSchema` (table map, file→table resolution) shared with ingest/
  ingest/                  – Registry-driven multi-target ingest (`master_ingest` binary):
    mod.rs                 – Ingester: targets, coalesced per-region triggers, cron reconcile
    run.rs                 – One region run: state diff, fetch+parse each changed file once,
                             fan-out to per-target writer tasks, checks, commit
    target.rs              – Per-target SQL: state tables, advisory lock, introspection,
                             mapping hash, staged upsert/delete of a file, raw rows
    registry_client.rs     – current / blob fetch with retries and streamed SHA-256 check
    http.rs                – /health, publish webhook, allow-shrink
  upstream.rs              – RegionRouter: multi-upstream routing with priority-ordered
                             targets (local client + remote Haruki nodes), circuit breakers
  api/
    routes.rs              – Axum router (health, image, protected API, internal routes)
    apis.rs                – Handler functions (profile, system, ranking proxies)
    middleware.rs           – JWT auth middleware with Redis caching
    image.rs               – Image/blob proxies (MySekai, housing/profile-card
                             thumbnails, custom music score) via RegionRouter
    internal.rs            – Node-to-node /internal/* API (sekai-api relay, login probe,
                             game byte stream, master version/bundle, update webhook,
                             app-identity push); gated by backend.internal_token
  client/
    sekai_client.rs        – Core game client (login, encrypted API calls, retry)
    account.rs             – CP and Nuverse account types, SekaiAccount trait
    session.rs             – AccountSession with per-account API locking
    helper.rs              – CookieHelper, VersionHelper, version comparison
    token_utils.rs         – JWT / Nuverse token user ID extraction
    nuverse_schema.rs      – Schema-bundle-driven array→dict restoration for Nuverse servers
  crypto/
    sekai_cryptor.rs       – AES-128-CBC encryption with MessagePack serialization
  db/
    mod.rs                 – init_db, init_master_db, init_redis
    entity/                – SeaORM entities: sekai_users, sekai_user_servers,
                             registry_state, registry_publish_history
  updater/
    scheduler.rs           – Cron jobs: cookie refresh, master update (local or
                             remote-account), app hash, master sync poll
    master.rs              – MasterUpdater: version check, download, git push, DB ingest
    sync.rs                – MasterSyncer: pull master bundles from an owner node
                             (webhook-triggered, cron fallback); records the last git
                             push outcome
    git.rs                 – GitHelper: stage, commit, push via git2; fetches before
                             every push and refuses to commit when the remote diverged
    master_stream.rs       – Table-by-table streaming decode of a downloaded master
                             payload (rows streamed one at a time)
    prune.rs               – Stale master file pruning after a complete dump/bundle:
                             protect list, ratio + absolute cap guards, producer-side
                             two-consecutive-dumps rule (pending set outside the worktree)
    apphash.rs             – AppHashUpdater: poll file/URL sources for new app hashes
  registry/
    service.rs             – MasterRegistry: per-region pull via MasterSyncer, git push,
                             ingest, per-region manifest publication
    http.rs                – Registry HTTP surface (pointers, digest-addressed blobs and
                             manifests, /health, app identity, subscriber fan-out)
    metas.rs               – music_metas feed (omakase rows injected)
    state.rs               – Per-region registry state (manifests, snapshots, history,
                             app identity, metas pointers): JSON files, or a database
                             when `registry.state_dsn` is set (one-time file import)
    blobs.rs               – Master file content store (`registry.blob_store`): the
                             master directories (fs) or content-addressed zstd blobs in
                             `registry_blobs` (pg): import, GC, directory fallback
  models/                  – ~92 auto-generated game data model files (never hand-edit;
                             regenerate them from the source data)
  bin/
    run_ingest.rs          – Standalone CLI for master data ingestion
    master_registry.rs     – Master data manager (registry), runs on the same config file
    master_ingest.rs       – Registry-driven ingest role (`ingest` config section)
    bench_profile.rs       – Per-stage latency benchmark for the profile proxy path
tools/
  ent_generator/           – Rust tool that reads src/models/ and generates:
                             - schema_info_generated.json (table→column mapping)
                             - ent_schemas/generated/*.go (EntGo schemas with table name annotations)
  nuverse_schema_generator/ – C# (Mono.Cecil) tool that reads Assembly-CSharp.dll from an
                             Il2Cpp DummyDll dump and emits Data/structures/nuverse_schema_bundle.json
docs/
  nuverse-schema-guide.md  – Nuverse schema assets: layout, field naming, update workflow
  master-registry-storage-and-ingest.md – registry content in PostgreSQL (blob store) and the
                             registry-driven multi-target ingest design
Data/master/               – Regional master data JSON files (jp, en, tw, kr, cn)
Data/registry/             – Registry JSON state (per-region manifests); music_metas blobs
Data/structures/           – Committed Nuverse schema assets (nuverse_schema_bundle.json, *.avsc)
schema_info.json           – Authoritative DB schema used by ingest engine
haruki-sekai-configs.example.yaml – Configuration template
```

## Key Concepts

### Server Regions & Protocols
- **CP servers** (Jp, En): Use cookies, JWT credentials, `PUT` login, direct AES-encrypted responses
- **Nuverse servers** (Tw, Kr, Cn): Use access tokens, `POST` login, CDN-based master data, flat array responses that need restoration
- Branch with `ServerRegion::is_cp_server()`

### Data Flow (Game API Call)
1. `RegionRouter` picks a target in ascending priority order (local client at
   `local_priority`, remote upstreams from `upstreams`), skipping targets with
   an open circuit breaker
2. Local target: select account session (round-robin), serialize request body →
   MessagePack → AES-128-CBC encrypt, send HTTP with session token + headers
3. Remote target: forward via the peer's `POST /internal/sekai-api` (the peer
   executes on its local client only, so forwarding cannot loop)
4. Receive encrypted response → AES decrypt → MessagePack → JSON
5. Return ordered JSON to API caller

### Multi-Node Topology
- `backend.internal_token` gates all `/internal/*` node-to-node endpoints
  (Bearer auth; endpoints answer 404 when the token is unset)
- Game API failover: per-region `upstreams` list of remote Haruki nodes; a
  region with upstreams but `enabled: false` (no local accounts) is served
  remote-only
- Master sync: each region has one owner node running the master updater; peer
  nodes configured with `master_sync.source_url` pull the owner's master bundle
  (`MasterSyncer`), triggered by the owner's `/internal/master-updated` webhook
  with `poll_cron` as fallback
- Remote-account master production: a node with `master_remote_source.url` runs
  the region's master pipeline locally but borrows a peer's accounts for the
  login probe and (CP) the encrypted master-split fetch, relayed as untouched
  bytes via `/internal/game-stream`
- App identity is not polled on a node: it arrives via `POST /internal/app-identity`
  (`src/api/internal.rs`)

### Master Data Pipeline
1. `MasterUpdater` checks game server for new data version
2. Downloads and decrypts master data (CP: split API; Nuverse: CDN + structure file)
3. Saves JSON files to `Data/master/{region}/master/`
4. Optionally pushes to git repository
5. Optionally ingests into PostgreSQL via `IngestionEngine`
6. `IngestionEngine` maps JSON filenames → table names using `schema_info.json`

- On an asset version change the updater first calls every `asset_updater_servers` entry
  whose `regions` list is empty or contains the region (awaited before the master download).
  A 409 containing "is disabled" (node does not own the region) is not retried; other 409s
  mean a job is running and retry `ASSET_UPDATER_MAX_CONFLICT_RETRIES`x at 60 s
- Downloaded payloads are decoded table by table with rows streamed (`master_stream.rs`),
  so the producer's peak memory is bounded by one row rather than the whole payload.
  Keep new master consumers on that path — never `unpack_ordered` a whole master
- Ingestion streams row batches bounded by row count *and* size (`ROWS_PER_BATCH`,
  `BATCH_BYTES` in `ingest_engine.rs`: a few master files have ~1k rows of 25-50 KB each);
  `master_database.ingest_concurrency` / `ingest.parse_concurrency` bound its memory.
  `tests/ingest_memory.rs` asserts the peak heap of a large-file ingest (PostgreSQL-gated)
- After a complete dump (every split decoded) or a complete bundle unpack, `*.json` files the
  dump did not produce are deleted before ingest and git push (`updater/prune.rs`); a failed or
  partial run never prunes. The producer deletes only after two consecutive complete dumps miss
  a table; both sides honour `BUILTIN_PROTECTED_TABLES` + `prune_protect`, `prune_min_ratio` and
  `prune_max_files`. Keep the protect list in sync with consumers' required tables. Ingest leaves
  the DB rows of a pruned table in place

### Master Registry
- The `master_registry` binary is the authoritative master data source other projects
  consume: it pulls each region from its owner via `MasterSyncer`, owns git push and
  ingest, and publishes per-region manifests (`contentHash` + `gitCommit`). State lives
  in `Data/registry/` JSON files, or in the `registry_state` / `registry_publish_history`
  tables when `registry.state_dsn` is set (files imported once into empty tables, kept).
  Run exactly one registry per state database (import/publish are not multi-writer safe;
  current manifests, app identity and metas pointers are cached in memory and refreshed
  only by that instance's own writes). Tables are created `IF NOT EXISTS`; there is no
  migration path, so a column change needs hand-written DDL. Rolling back to files: stop
  the registry, delete `<state_dir>/manifests/*/current.json` (startup `publish_missing`
  skips regions that already have a current, which would serve the pre-DB manifest), clear
  `state_dsn`, restart
- Master file content is served from the master directories (`registry.blob_store: fs`,
  default) or from `registry_blobs` in the state database (`pg`, requires `state_dsn`):
  zstd blobs keyed by SHA-256, stored before the manifest that lists them commits (the
  publish transaction checks), served decompressed with the same bytes and headers
  (`Last-Modified` = first stored). `blob/` then also serves retained snapshots' files;
  `files/` and `bundle` follow `current`. Existing manifests are imported in the background
  at startup (disk, else `git cat-file` at the manifest's `gitCommit`), reads fall back to
  disk while a blob is missing or the database is slow/down (2 s budget per read, then a
  5 s circuit breaker), and GC removes blobs no current/retained snapshot lists after
  `blob_gc_grace_secs` (min 300; GC aborts if any snapshot is unreadable). The state pool is
  8 connections with `pg`, 4 otherwise. See `docs/master-registry-storage-and-ingest.md`
- It also maintains the music_metas feed (`metas.rs`, omakase rows injected), serves the
  app identity (`GET /v1/app/{region}`; `PUT` stores an override and pushes it to
  `registry.account_nodes`), and fans `master-updated` notices out to
  `registry.subscribers` (body `{server, dataVersion, contentHash, gitCommit, changedFiles,
  removedFiles}`; the last four were added compatibly, receivers must still re-read `current`)
- `/health` reports `status: degraded` when the last git push failed
- CDN contract: pointers (`current`, `files/{name}`, `music_metas.json`, `app`) are
  `no-cache` + ETag; digest-addressed `blob/{sha256}` and `manifests/{hash}` are
  immutable — never serve changing bytes under a digest URL; their 404s/503s are `no-store`

### Registry-Driven Ingest (`master_ingest`)
- Opt-in role: runs only as its own process with an `ingest` config section; the in-sync path
  (`master_database` on the syncer/registry/updater) is unchanged and stays the default. Never
  point both at the same database
- Reads only the registry (`/health` must report `blobStore: pg`, then `current` and
  `blob/{sha256}`); the webhook (`POST /internal/master-updated`, bearer `ingest.webhook_token`)
  and the cron only trigger a reconcile of the registry's `current`
- Per target (`ingest.targets[]`, state keyed by target `name`): `master_ingest_state` (per-file
  sha256 + mapping hash, rows, missing_since), `master_ingest_version`, `master_raw` (only with
  `raw: true`, opt-in), `master_ingest_allow_shrink`. A file is rewritten when its sha256 or mapping
  hash (schema entry, DB columns/types, key, `raw`, `target::ENGINE_VERSION`) changed — bump
  `ENGINE_VERSION` when value conversion or write SQL changes
- Each file is staged into a temp table, then upserted `ON CONFLICT (key) … WHERE … IS DISTINCT FROM`
  and stale keys deleted (keyless tables / no matching unique index: delete + insert). Steady runs
  commit a region in one transaction per target; runs above `stage_tables`/`stage_bytes` commit
  per file (status `staging`). Checks: min_ratio (override: allow-shrink), required tables, never
  backwards; removed files are cleared after two consecutive missing versions, never protected ones
- DB tests: `HARUKI_TEST_INGEST_DSN` (a user that may CREATE DATABASE), optional
  `HARUKI_TEST_INGEST_MASTER_DIR` for a full-directory equivalence run

### Schema System
- `schema_info.json` defines table names, column types, and unique keys
- Generated by `tools/ent_generator` from Rust model files
- Table names use lowercased type aliases (e.g., `MusicArtist` → `musicartists`)
- Column `id` in JSON is mapped to `game_id` in DB (avoids PK collision)
- Every table has a `server_region` column for multi-region data

## Coding Standards

### Language & Framework
- Rust 2021 edition, async with Tokio
- Axum for HTTP, SeaORM for database, tracing for logging
- sonic-rs for fast JSON, serde_json when key order matters

### Error Handling
- Define errors as `AppError` variants in `src/error.rs`
- Implement `From<ExternalError>` for automatic `?` conversion
- `AppError` implements `IntoResponse` with JSON error body: `{"result":"failed","status":N,"message":"..."}`
- Use `anyhow::Result` only in `main()` and CLI binaries, not in library code

### Naming & Style
- Modules/files: `snake_case` (e.g., `sekai_client.rs`)
- Structs/enums: `PascalCase`
- Functions: `snake_case`
- Game data models: all fields `Option<T>`, `#[serde(rename_all = "camelCase")]`
- Enum variants for game types: `#[serde(rename_all = "snake_case")]`
- Config enums: `#[serde(rename_all = "lowercase")]`
- Comments: only when clarifying non-obvious logic; do not over-comment

### Concurrency
- `Arc<RwLock<>>` (parking_lot) for shared mutable state
- `tokio::sync::Mutex` for async critical sections
- `AtomicBool` / `AtomicUsize` for lock-free flags and counters
- Parallel initialization via `tokio::spawn` + `futures::future::join_all`

### Logging
- Use `tracing::{info, warn, error, debug}` — never `println!` in library code
- Region-specific messages: prefix with `"{} message", region.as_str().to_uppercase()`
- Log levels: `error` for unrecoverable failures, `warn` for handled failures, `info` for state changes, `debug` for diagnostics

## Testing

- Tests live in `#[cfg(test)] mod tests` blocks within source files
- Async tests use `#[tokio::test]`
- Tests requiring external services (DB, Redis, game servers) are marked `#[ignore]`
- Return type: `anyhow::Result<()>` for tests with fallible operations
- No separate `tests/` directory; all tests are inline

## Building

```bash
# Development build
cargo build

# Release build (with LTO, stripped)
cargo build --release

# Run server
cargo run

# Run master data ingestion
cargo run --bin run_ingest

# Run the master data manager (registry) on the same config file
cargo run --bin master_registry

# Run the registry-driven ingest role (needs an `ingest` section)
cargo run --bin master_ingest

# Tests, a single test, and the ones needing external services
cargo test
cargo test <test_name>
cargo test -- --ignored

# Lint and format
cargo clippy
cargo fmt

# Run ent_generator (from tools/ent_generator/)
cd tools/ent_generator && cargo run

# Docker build
docker build --build-arg VERSION=v1.0.0 -t haruki-sekai-api .
```

## Common Tasks for Agents

### Adding a New Game Data Model
1. Create `src/models/{name}.rs` with `pub type X = Vec<XElement>;` and the struct definition
2. Add `pub mod {name};` to `src/models.rs`
3. Run `cd tools/ent_generator && cargo run` to regenerate `schema_info_generated.json` and Go schemas
4. Copy `schema_info_generated.json` to `schema_info.json` if correct
5. Copy generated Go schemas to the EntGo project and run migrations

### Adding a New API Endpoint
1. Add handler function in `src/api/apis.rs`
2. Register route in `src/api/routes.rs` (protected routes go under the auth middleware layer)
3. If the endpoint proxies game API calls, use `proxy_game_api()` or model after existing handlers

### Adding a New Error Variant
1. Add variant to `AppError` enum in `src/error.rs`
2. Add `#[error("...")]` message
3. Add HTTP status mapping in `status_code()` method
4. Add `From` implementation if converting from an external error type

### Modifying the Ingest Engine
- Table-to-file mapping: `resolve_table_name()` normalizes filenames (lowercase, strip underscores, try plural forms)
- Column mapping: JSON keys are normalized (lowercase + strip underscores), `id` → `game_id`
- Unmapped JSON keys are dropped (only keys matching a schema column are inserted)
- In-sync path: one transaction per file, DELETE existing region data, then batch INSERT
- Registry-driven path (`src/ingest/`): see "Registry-Driven Ingest" above; unmapped keys are
  counted in `master_ingest_version.unknown_keys` and kept in `master_raw` when `raw: true`

### Modifying Config
The config file is `haruki-sekai-configs.yaml`, located via the `CONFIG_PATH` env var
(defaults to the current directory). Per-region server entries hold AES keys (hex),
account directories, master data paths and cron schedules.

1. Add field to relevant struct in `src/config.rs` with `#[serde(default = "...")]`
2. Add default function if needed
3. Update `haruki-sekai-configs.example.yaml`

## Git commits

All commit subjects must follow:

```text
[Type] Short description starting with capital letter
```

Allowed types:

| Type      | Usage                                                 |
|-----------|-------------------------------------------------------|
| `[Feat]`  | New feature or capability                             |
| `[Fix]`   | Bug fix                                               |
| `[Chore]` | Maintenance, refactoring, dependency or build changes |
| `[Docs]`  | Documentation-only changes                            |

Rules:

- Description starts with a capital letter.
- Use imperative mood: `Add ...`, not `Added ...`.
- No trailing period.
- Keep the subject at or below roughly 70 characters.
- **Agent attribution uses the standard Git `Co-authored-by:` trailer in the commit body, not a free-form `Agent:` line.** This makes GitHub render the co-author avatar on the commit page. The trailer must be on its own line, separated from the subject by a blank line, in the form `Co-authored-by: <Display Name> <email>`. Suggested values per agent:
  - Claude: `Co-authored-by: Claude Fable 5 <noreply@anthropic.com>` (substitute the actual model, e.g. `Claude Opus 4.7`, `Claude Sonnet 4.6`, `Claude Haiku 4.5`)
  - Codex: `Co-authored-by: Codex <noreply@openai.com>`
  - Copilot: `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`

Examples from this repo's history:

```text
[Feat] Add custom music score proxy routes
[Fix] Replace manual padding repeat
[Chore] Update dependencies
[Chore] Bump actions/download-artifact from 4 to 8
```

## GitHub Actions workflows

Use the standardized workflow layout in `.github/workflows`:

- `ci.yml` runs on `main` pushes, pull requests targeting `main`, and manual dispatch.
- Rust CI order: `cargo fmt --all -- --check`, `cargo check --locked --all-targets`, `cargo clippy --locked --all-targets -- -D warnings`, then `cargo test --locked`.
- `release.yml` is the standard release build entrypoint. It runs on `v*` tags and manual dispatch, builds release artifacts, uploads them with `actions/upload-artifact`, and publishes GitHub Release assets on tag pushes.
- `docker.yml` is the standard Docker entrypoint. It runs on `main` pushes, `v*` tags, PRs that touch Docker/build inputs, and manual dispatch. PRs build only; non-PR runs push GHCR images with lowercase image names and Docker metadata tags.

Workflow maintenance rules:

- Keep workflow filenames and top-level names aligned: `CI`, `Release`, `Docker`, and optional package-specific names.
- Use `actions/checkout@v7`, `actions/upload-artifact@v7`, `actions/download-artifact@v8`, `softprops/action-gh-release@v3`, and current Docker actions (`setup-buildx@v4`, `login@v4`, `metadata@v6`, `build-push@v7`).
- Keep `permissions` minimal: `contents: read` for CI/Docker build-only work, `contents: write` for release publishing, and `packages: write` only when pushing container images.
- Use workflow `concurrency` keyed by workflow name and ref, with release jobs using `release-${{ github.ref_name }}` and `cancel-in-progress: false`.
- Do not reintroduce legacy workflow names such as `rust-ci.yml`, `build.yml`, `release-build.yml`, `docker-build.yml`, or `docker-release.yml` unless a package-specific workflow already exists and is intentionally preserved.

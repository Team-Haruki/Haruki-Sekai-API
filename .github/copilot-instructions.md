# Copilot Instructions for Haruki Sekai API

## Project Overview

Haruki Sekai API is a Rust companion service for HarukiBot, providing proxied API access to multiple regional servers of the game "Project Sekai: Colorful Stage". It handles encrypted communication with game servers, master data management, and user authentication.

## Tech Stack

- **Language**: Rust 2021 edition (minimum 1.70)
- **Async Runtime**: Tokio (full features)
- **Web Framework**: Axum 0.8
- **ORM**: SeaORM 2.0 (supports SQLite, MySQL, PostgreSQL)
- **Caching**: Redis with async connection manager
- **Serialization**: sonic-rs (primary JSON), serde_json (ordered), rmp-serde (MessagePack)
- **Encryption**: AES-128-CBC via `aes` + `cbc` crates
- **Auth**: JWT via `jsonwebtoken` (HS256)
- **Logging**: `tracing` + `tracing-subscriber` with env-filter
- **Error Handling**: `thiserror` (AppError enum) + `anyhow` (main bootstrap)
- **Git**: `git2` (libgit2 bindings, vendored OpenSSL)
- **Scheduling**: `tokio-cron-scheduler`

## Architecture

```
src/
├── main.rs              # Server bootstrap, graceful shutdown
├── lib.rs               # Public module exports, AppState definition
├── config.rs            # YAML config with serde defaults
├── error.rs             # AppError enum (thiserror), HTTP status mapping
├── utils.rs             # Retry logic, CachedResource<T>
├── ingest_engine.rs     # Master data JSON → DB ingestion
├── api/                 # HTTP layer (Axum)
│   ├── routes.rs        # Router definition, health check
│   ├── apis.rs          # Endpoint handlers (proxy to game API)
│   ├── middleware.rs     # JWT auth middleware, Redis caching
│   └── image.rs         # MySekai image proxy endpoint
├── client/              # Game server communication
│   ├── sekai_client.rs  # Main client: login, API calls, retry, encryption
│   ├── account.rs       # Account types: CP (JWT) and Nuverse (access token)
│   ├── session.rs       # Session management with API locking
│   ├── helper.rs        # CookieHelper, VersionHelper
│   ├── token_utils.rs   # JWT/token extraction utilities
│   └── nuverse.rs       # Nuverse response array→dict restoration
├── crypto/
│   └── sekai_cryptor.rs # AES-128-CBC encrypt/decrypt with MessagePack
├── db/
│   ├── mod.rs           # init_db, init_master_db, init_redis
│   └── entity/          # SeaORM entities (sekai_users, sekai_user_servers)
├── updater/
│   ├── scheduler.rs     # Cron job registration
│   ├── master.rs        # Master data version check & download
│   ├── git.rs           # Git commit & push via git2
│   └── apphash.rs       # App hash polling from file/URL sources
├── models/              # ~84 auto-generated game data models
│   └── *.rs             # Each: pub type X = Vec<XElement>; with camelCase serde
└── bin/
    └── run_ingest.rs    # Standalone ingestion CLI tool
```

## Server Regions

Five regions with two server protocols:

| Region | Enum | Protocol | Key Difference |
|--------|------|----------|----------------|
| Japan | `Jp` | ColorfulPalette (CP) | Uses cookies + JWT credential |
| English | `En` | ColorfulPalette (CP) | Uses cookies + JWT credential |
| Taiwan | `Tw` | Nuverse | Uses access tokens, CDN versioning |
| Korea | `Kr` | Nuverse | Uses access tokens, CDN versioning |
| China | `Cn` | Nuverse | Uses access tokens, CDN versioning |

Use `ServerRegion::is_cp_server()` to branch on protocol differences.

## Coding Conventions

### Error Handling
- Use `AppError` variants for all domain errors (defined in `src/error.rs`)
- Use `?` operator with `From` implementations for external crate errors
- Use `anyhow::Result` only in `main()` and standalone binaries
- Implement `IntoResponse` for HTTP error responses with JSON body

### Async Patterns
- Use `tokio::spawn` for parallel initialization
- Use `Arc<RwLock<>>` for session management (parking_lot where sync is needed)
- Use `tokio::sync::Mutex` for async critical sections (e.g., API call serialization)
- Use `AtomicBool` / `AtomicUsize` for lock-free coordination

### Serialization
- Game API models: `#[serde(rename_all = "camelCase")]`
- Enum variants: `#[serde(rename_all = "snake_case")]`
- Config enums: `#[serde(rename_all = "lowercase")]`
- All model fields are `Option<T>` (game data may be incomplete)
- Use `sonic_rs` for performance-critical JSON; `serde_json` when key order matters

### Logging
- Use `tracing::{info, warn, error, debug}` macros
- Prefix region-specific logs with `region.as_str().to_uppercase()`
- No file/line info in logs; use custom ISO-8601 timestamp formatter

### Database
- SeaORM with derive macros for entities
- Two separate databases: user DB (`database`) and master data DB (`master_database`)
- Tables created via `create_table_from_entity().if_not_exists()`
- Master data tables defined in `schema_info.json`, ingested dynamically

### Models
- Auto-generated from game data schemas
- Pattern: `pub type ModelName = Vec<ModelElement>;` with `#[serde(rename_all = "camelCase")]`
- Located in `src/models/`, one file per game data table
- Do not manually edit model files; regenerate from source data

## Key Configuration

- Config file: `haruki-sekai-configs.yaml` (loaded via `CONFIG_PATH` env var)
- Schema definition: `schema_info.json` (maps JSON files to DB tables)
- AES keys: Per-region hex-encoded 128-bit key + IV
- Accounts: JSON files in per-region `account_dir` directories

## Tools

### ent_generator (`tools/ent_generator/`)
- Reads Rust model files from `src/models/`
- Generates `schema_info_generated.json` (table names, columns, types, unique keys)
- Generates EntGo schema Go files in `ent_schemas/generated/` with explicit `entsql.Annotation{Table: "..."}` to ensure DB table names match `schema_info.json`
- Run from `tools/ent_generator/`: `cargo run`

### run_ingest (`src/bin/run_ingest.rs`)
- Standalone binary for bulk-ingesting master data into PostgreSQL
- Reads from `Data/master/*/master/*.json`
- Uses `schema_info.json` for column mapping
- Run: `cargo run --bin run_ingest`

## Build & Release

- Release profile: LTO enabled, single codegen unit, strip symbols, abort on panic
- Docker: Multi-stage cargo-chef build (rust:1.98-alpine → alpine:3.24), exposes ports 9999, 9998 and 9997
- CI: shared `seiunx-dev/ci-templates` workflows (see below); images pushed from `main`, binaries built and the main image promoted on `v*` tags
- Release archives: linux-x64, macos-arm64, windows-x64; Docker image: linux/amd64

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
  - Claude (any 4.x): `Co-authored-by: Claude Opus 4.7 <noreply@anthropic.com>` (substitute the actual model, e.g. `Claude Sonnet 4.6`, `Claude Haiku 4.5`)
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

CI and releases reuse the shared workflows in
[seiunx-dev/ci-templates](https://github.com/seiunx-dev/ci-templates) (`@v1`). The files in
`.github/workflows` are thin callers; see `docs/ci-workflows.md` for the job-by-job layout.

- `ci.yml` (`CI`) runs on `main` pushes, pull requests targeting `main`, and manual dispatch: Rust lint (fmt + Clippy `-D warnings` for the crate and `tools/ent_generator`), tests run once under cargo-llvm-cov with a Postgres service (plus the PG-gated `--ignored ingest::tests postgres` tests), an MSRV check against `rust-version` in `Cargo.toml` (1.94), Python tool tests, Sonar (scans the uploaded coverage, no second test run), Docker, and workflow lint (actionlint).
- `CI OK` is the only required status check; it fails when any CI job fails or is cancelled.
- Docker (inside `ci.yml`): PRs build only, and only when Docker inputs change. On `main` the image job does not wait for the tests: it runs in parallel and pushes the immutable `ghcr.io/team-haruki/haruki-sekai-api:sha-<full sha>` and `:sha-<7 chars>` as soon as the build finishes; the `Docker tags` job (template `docker-retag.yml`, after `CI OK`) then moves `:main` to that digest without rebuilding. `:main` therefore only follows commits whose `CI OK` passed and lags the `:sha-*` tags until then; deploy `:sha-<7 chars>` when the image is needed earlier.
- `release.yml` (`Release`) runs on `v*` tags and manual dispatch. A manual dispatch is a dry run: it builds the archives and publishes nothing.
- Release flow: bump `version` in `Cargo.toml` in a PR → merge → wait for `CI OK` on main → push tag `v<version>` → the gate checks tag == `v` + Cargo version and waits for `CI OK` on the tagged commit → linux-x64 / macos-arm64 / windows-x64 archives are built on the tag (asset names `haruki-sekai-api-<label>.tar.gz|zip`, plus `SHA256SUMS-<tag>.txt`) → the main `:sha-<sha>` image is re-tagged as `:X.Y.Z`, `:X.Y`, `:latest` without a rebuild → the GitHub Release is published. Never rewrite the version from the tag.
- Caches: Rust caches and the Docker registry cache (`:buildcache`) are written only from `main`; PRs and tags only read them. Release builds are cold builds on the tag.
- Concurrency: PR runs are grouped per PR and cancel older runs; every other event gets its own group per commit, so main runs are never cancelled.

Workflow maintenance rules:

- Reuse the shared templates first. Add a custom job or step only when a template genuinely cannot meet this project's needs; keep it in the thin caller (`ci.yml` / `release.yml`) with a comment explaining why the template was not enough.
- Fix template bugs and add missing template features upstream in `seiunx-dev/ci-templates` instead of working around them here; callers stay on `@v1`.
- Keep `permissions` minimal: `contents: read` by default, `packages: write` only on the Docker jobs, `contents: write` only on the GitHub Release job.
- Pin third-party actions in custom steps to a commit SHA with a version comment.
- Do not reintroduce `docker.yml`, `sonar.yml`, per-workflow prebuild/reuse logic, or main-push release builds.
- Do not add `needs:` on the test jobs to the Docker job, and do not suppress `githubactions:S7637` (full-SHA pins) in `sonar-project.properties`: the template's `sonar.yml` already ignores it for the `@v1` references.

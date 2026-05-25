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
- Docker: Multi-stage build (rust:1.92-alpine → alpine:3.22), exposes port 9999
- CI: GitHub Actions on tag push (`v*`) for cross-platform binaries and Docker images
- Targets: linux-amd64, linux-arm64, windows-amd64, macos-amd64, macos-arm64

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

Use the standardized workflow layout in `.github/workflows`:

- `ci.yml` runs on `main` pushes, pull requests targeting `main`, and manual dispatch.
- Rust CI order: `cargo fmt --all -- --check`, `cargo check --locked --all-targets`, `cargo clippy --locked --all-targets -- -D warnings`, then `cargo test --locked`.
- `release.yml` is the standard release build entrypoint. It runs on `v*` tags and manual dispatch, builds release artifacts, uploads them with `actions/upload-artifact`, and publishes GitHub Release assets on tag pushes.
- `docker.yml` is the standard Docker entrypoint. It runs on `main` pushes, `v*` tags, PRs that touch Docker/build inputs, and manual dispatch. PRs build only; non-PR runs push GHCR images with lowercase image names and Docker metadata tags.

Workflow maintenance rules:

- Keep workflow filenames and top-level names aligned: `CI`, `Release`, `Docker`, and optional package-specific names.
- Use `actions/checkout@v6`, `actions/setup-go@v6`, `actions/upload-artifact@v7`, `actions/download-artifact@v8`, `softprops/action-gh-release@v3`, and current Docker actions (`setup-buildx@v4`, `login@v4`, `metadata@v6`, `build-push@v7`).
- Keep `permissions` minimal: `contents: read` for CI/Docker build-only work, `contents: write` for release publishing, and `packages: write` only when pushing container images.
- Use workflow `concurrency` keyed by workflow name and ref, with release jobs using `release-${{ github.ref_name }}` and `cancel-in-progress: false`.
- Do not reintroduce legacy workflow names such as `rust-ci.yml`, `build.yml`, `release-build.yml`, `docker-build.yml`, or `docker-release.yml` unless a package-specific workflow already exists and is intentionally preserved.

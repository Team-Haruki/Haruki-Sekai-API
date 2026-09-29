# CI and release build ownership

## Investigation (2026-09-30)

The Actions API reported 133 caches totaling 11.32 GiB: about 6.87 GiB of
BuildKit cache and 4.45 GiB of Rust cache. Much of this was stored under PR
merge refs and individual tag refs. Active uploaded artifacts were another
1.03 GiB (259 artifacts). These are separate storage categories.

For v6.27.0, the Docker build log showed dependency recompilation in the
single `RUN cargo build --release --locked` layer taking 10m58s. The same
source was built for the PR, main push, and tag push. Sonar had no compiled
coverage cache. Native release caches were written under isolated tag refs,
while the main branch never prepared native release binaries.

## Responsibilities

| Workflow | PR | main | Version tag |
| --- | --- | --- | --- |
| CI | Format, check, Clippy, Rust and Python tests | Same checks; save shared Rust cache | — |
| SonarQube | Coverage + PostgreSQL tests + analysis; restore cache | Same analysis; save instrumented dependency cache | — |
| Docker | Full image validation; read shared registry cache | Build/push commit image and refresh registry cache | Reuse the exact main commit image; build if unavailable |
| Release | — | Prepare three platform archives for relevant changes; save platform caches | Publish the exact main commit archives; build if absent/expired |

The standard workflow filenames are retained. PR image validation has only
read permissions; publishing jobs alone receive `packages: write` or
`contents: write`. Tag and package versions must match. Docker images retain
the binary's Cargo version in their OCI label even when built on main.

`tools/ci_reuse.py` selects only successful **main push** runs with exactly the
release commit SHA. A tag waits up to 30 minutes for an in-progress matching
build. Failed builds stop publication. Missing/cancelled builds and expired
archive sets fall back to a fresh build. Downloaded archives come from the
validated run ID; image promotion additionally verifies the revision and
version labels, then pins its source digest. A label mismatch fails rather
than silently promoting the wrong image.

Docker uses cargo-chef 0.1.78 with the same Rust base for planning and building.
Its recipe isolates dependency compilation (including masking local package
versions) from application source changes. Only Cargo inputs, `src`, the
schema and committed structures enter the build context. Registry cache
`ghcr.io/team-haruki/haruki-sekai-api:buildcache` replaces the per-ref GHA
BuildKit cache; only main writes it. PRs and tags do not save large Rust caches.
Sonar caches `target/llvm-cov-target` separately and instruments library tests;
normal CI still runs all-target checks and the full default test suite.

Native archives contain the API, registry, ingester, standalone ingest command
and `schema_info.json`. Benchmark executables are not release targets. Actions
archives and Docker build records have a seven-day retention; published GitHub
Release assets are independent of that temporary retention.

The first build must populate the new caches. Subsequent build times must be
measured on Actions; dependency-layer reuse and tag promotion eliminate work,
but cold-build speed is not promised. Existing caches/artifacts are not removed
by these workflows; obsolete GHA BuildKit caches can be reclaimed after the
new main registry cache is verified.

References: [cargo-chef](https://github.com/LukeMathWalker/cargo-chef),
[Rust cache](https://github.com/Swatinem/rust-cache),
[LLVM coverage](https://github.com/taiki-e/cargo-llvm-cov).

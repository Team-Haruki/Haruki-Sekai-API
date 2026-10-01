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

## Responsibilities (since 2026-10-01: shared templates)

`ci.yml` and `release.yml` call the reusable workflows in
[seiunx-dev/ci-templates](https://github.com/seiunx-dev/ci-templates) at `@v1`.
`docker.yml` and `sonar.yml` were folded into `ci.yml`.

| Workflow / job | PR | main | Version tag |
| --- | --- | --- | --- |
| CI / Rust | fmt + Clippy (main crate and `tools/ent_generator`); tests once under cargo-llvm-cov with Postgres, incl. the ignored `ingest::tests postgres` tests; MSRV 1.94 `cargo check` | Same; saves the Rust caches | — |
| CI / Python tools | unittest + coverage | Same | — |
| CI / Sonar | Scans the uploaded coverage (no second test run) | Same | — |
| CI / Docker | Build only, when Docker inputs change; reads the registry cache | Push `:main`, `:sha-<sha>`, `:sha-<sha7>`; writes the registry cache | — |
| CI / CI OK | Single required check | Same | — |
| Release | — | — | Gate (tag == `v` + Cargo version, waits for `CI OK`), build linux/macos/windows archives, re-tag `:sha-<sha>` as `:X.Y.Z`/`:X.Y`/`:latest`, publish the GitHub Release |

A manual `Release` dispatch is a dry run: gate + archives only, nothing is
published. Asset names are unchanged (`haruki-sekai-api-<label>.tar.gz|zip`,
flat layout, plus `SHA256SUMS-<tag>.txt`). Main images carry
`org.opencontainers.image.version=main-<sha7>`; promoted release tags reuse
that image unchanged. `tools/ci_reuse.py` is no longer called by any workflow.

Docker uses cargo-chef 0.1.78 with the same Rust base for planning and building.
Its recipe isolates dependency compilation (including masking local package
versions) from application source changes. Only Cargo inputs, `src`, the
schema and committed structures enter the build context. Registry cache
`ghcr.io/team-haruki/haruki-sekai-api:buildcache` replaces the per-ref GHA
BuildKit cache; only main writes it. PRs and tags do not save large Rust caches.
Coverage is produced by the single CI test run and handed to Sonar as an artifact.

Native archives contain the API, registry, ingester, standalone ingest command
and `schema_info.json`. Benchmark executables are not release targets. Release
archive artifacts are kept for one day and Docker build records for three; published GitHub
Release assets are independent of that temporary retention.

The first build must populate the new caches. Subsequent build times must be
measured on Actions; dependency-layer reuse and tag promotion eliminate work,
but cold-build speed is not promised. Existing caches/artifacts are not removed
by these workflows; obsolete GHA BuildKit caches can be reclaimed after the
new main registry cache is verified.

References: [cargo-chef](https://github.com/LukeMathWalker/cargo-chef),
[Rust cache](https://github.com/Swatinem/rust-cache),
[LLVM coverage](https://github.com/taiki-e/cargo-llvm-cov).

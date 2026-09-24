# Master registry: content storage and registry-driven ingest

Status: part A (content in PostgreSQL) ships with this change; part B
(registry-driven ingest with multiple database targets) is a design for a
follow-up.

Context. The registry (`master_registry` binary, CN05) pulls each region from
its owner, writes the master JSON into the region's git worktree, pushes it to
GitHub (git stays the authoritative archive and the external distribution),
and publishes a per-region manifest (`contentHash`, `gitCommit`, per-file
`sha256`). Since 6.23.0 the registry *state* (current manifests, 20 snapshots
per region, publish history, app identity, music_metas pointers) can live in
PostgreSQL (`registry.state_dsn`). The master *content* was still read from the
worktrees on every request. The total current content is about 417 MB over five
regions; the largest file is `costume3ds.json` at about 55 MB. CN05 has about
665 MB of RAM free, so every path here streams and holds only a small, fixed
amount of data in memory at once.

---

## A. Master content in PostgreSQL (`registry.blob_store: pg`)

### Configuration

| key | default | meaning |
|---|---|---|
| `registry.blob_store` | `fs` | `fs`: serve from the master directories (current behaviour). `pg`: serve from `registry_blobs` |
| `registry.blob_gc_grace_secs` | `86400` | an unreferenced blob is deleted only after no publish has referenced it for this long |

`pg` requires `registry.state_dsn`, and startup fails without it. There is no
separate `blob_dsn`. The blob table has to be in the same database as the state
tables so a publish can check, inside its own transaction, that the new
`current` lists only stored blobs. Because the blobs are compressed (below),
the extra data is small enough for the state database.

The backend works on any SeaORM database. The tests run it on SQLite, and on
PostgreSQL when `HARUKI_TEST_REGISTRY_DSN` is set.

### Table

```sql
registry_blobs(
  sha256       varchar primary key,   -- digest of the uncompressed bytes (= manifest sha256)
  size         bigint,                -- uncompressed size (Content-Length)
  encoding     varchar,               -- 'zstd'
  stored_size  bigint,                -- compressed size
  content      bytea,
  created_at   timestamptz,           -- first stored (files/ Last-Modified)
  last_seen_at timestamptz            -- last publish/import that referenced it (indexed)
)
```

The table is content-addressed and shared by all regions and versions. A file
that did not change between versions, or that is identical across regions, is
stored once. Inserts are insert-if-absent (`ON CONFLICT (sha256) DO UPDATE SET
last_seen_at`).

### Compression

Each blob is one zstd frame: level 9, window log 20 (1 MiB), with a content
checksum. The encoder is already in the dependency tree through tower-http and
reqwest. Measured on a real 281 MB, 425-file region:

| setting | stored | ratio |
|---|---|---|
| zstd -3 | 9.8 MB | 29x |
| **zstd -9, wlog 20** | **8.0 MB** | **35x** |
| zstd -19 | 6.3 MB | 45x, 50x the CPU |
| gzip -6 | 14.5 MB | 19x |

**Estimate for CN05.** Current content is about 417 MB, which stores as about
12 MB. The 20 retained snapshots per region share almost all their files; a
master update usually touches a few dozen tables. Allowing 20 versions of the
changed tables gives a total of roughly **20–60 MB** in `registry_blobs`.
Compare this with 417 MB of worktree for current alone.

Responses are always decompressed on the fly. Clients get the same
uncompressed bytes they get today, with no `Content-Encoding`. Content-Encoding
passthrough, which would send the stored frame to clients that accept `zstd`,
is not implemented. A CDN would then hold per-encoding variants, and the bodies
would stop being identical to the fs store's.

### Publish flow

1. Build the manifest from the worktree (unchanged).
2. `store_manifest(Strict)`. One query lists which digests are already stored;
   those rows get `last_seen_at = now`. Each missing file is read once and
   streamed through SHA-256 and the zstd encoder together, one file at a time.
   The digest and size must equal the manifest's, otherwise the publish fails,
   because the file changed under the publish. The row is then inserted.
3. `state.publish` runs as one transaction (#105). It first checks that every
   digest the manifest lists is in `registry_blobs`, and refuses to commit
   otherwise. After that it switches `current`, writes the snapshot, prunes the
   snapshots and appends history. As a result, `current` never lists a blob that
   cannot be served.
4. Subscribers are notified, then one GC pass runs.

### Reads

| endpoint | fs (unchanged) | pg |
|---|---|---|
| `current`, `manifests/{hash}`, `history` | state | state |
| `files/{name}` | the file on disk (whatever is there now) | resolved through the current manifest, then the blob |
| `blob/{sha256}` | only files of the current manifest whose bytes on disk still hash to the digest | any stored blob: current, or listed by a retained snapshot of any region, or not yet collected |
| `bundle` | tar of the directory (readdir order, file metadata) | tar streamed from the blobs of `current` (manifest order, mode 0644, mtime = publish time) plus a version entry built from the manifest |

- **Byte identity.** For `current`, `files/`, `blob/` and the error bodies,
  the body, status, `ETag`, `Cache-Control`, `Content-Type` and
  `Content-Length` are identical between the two stores (tested). Two headers
  differ:
  - `Last-Modified` on `files/` is the blob's `created_at` instead of the file's
    mtime.
  - The bundle has the same entries with the same bytes, but different tar
    metadata and order. Its version entry is the canonical `VersionInfo` JSON,
    which parses to the same values. The fs bundle was not reproducible to begin
    with, since it uses readdir order and mtimes.
- **Historical blobs are served.** Serving a blob listed only by a retained
  snapshot is safe under the CDN contract, because a digest URL can only ever
  return the bytes with that digest. It also makes the blob URLs of the last 20
  versions keep working for pinned consumers. Blobs are not scoped per region: a
  digest identifies bytes, whichever region lists it.
- **Fallback.** A current file whose blob is missing, because the import is
  still running or the database cannot be read, is served from the worktree
  with the fs store's digest check. The bundle falls back to the directory tar
  while any blob is missing.
- **Memory.** At most 16 blob responses are in flight at once
  (`READ_CONCURRENCY`, a semaphore held for the whole stream). A read waits
  at most 10 s for a slot, so slow clients cannot stall the others; after
  that, a current file is served from disk, and any other read fails with an
  error. The 37 MB file stores as 0.85 MB, so the largest
  blob is about 1.3 MB, and each decoder has a 1 MiB window. The worst case is
  therefore about 40 MB. The bundle
  is written by a blocking task into a bounded channel of eight 64 KiB chunks,
  one blob at a time, with no temp file. The state pool grew from 4 to 8
  connections. At most 4 blob row queries run at once, so state writes always
  find a connection. A blob holds a connection only for its single-row fetch,
  not while the body streams. No extra
  server-side cache is added: blob URLs are immutable and EdgeOne caches them,
  `files/` is revalidated with 304s, and decompressing 1.5 MB takes about 2 ms.

### Import (fs → pg on an existing deployment)

At startup the registry runs `publish_missing` (unchanged), then imports in the
background (`Registry::import_blobs`):

- For each region, it takes the current manifest and then the retained
  snapshots, newest first, and stores their missing blobs with
  `store_manifest(Lenient)`.
- A file comes from the worktree when its bytes still match the digest.
  Otherwise it comes from the manifest's git commit (`git cat-file blob
  <gitCommit>:./<name>`, streamed). If neither has it, it is skipped with a
  warning. Snapshots from before `gitCommit` existed only have the worktree.
- The import is idempotent: every run skips stored digests, and the publish
  lock is held per manifest.
- It holds one file in memory at a time. Measured with the release build on a
  real 281 MB, 425-file region against PostgreSQL 17:
  - The import took 2.9 s. It stored 407 blobs (18 duplicate files were
    deduplicated), 281 MB → 7.95 MB.
  - Peak RSS of the whole process was 53.5 MB. That covers the import plus 8
    concurrent full bundles and 8 concurrent 37 MB blob downloads afterwards.
  - One full bundle streamed in 0.5 s.

Reads keep working during the import through the directory fallback.

### Garbage collection

After every changed publish, and after the import, one pass runs:

- It computes the referenced set: every digest in the current manifest and the
  retained snapshots of all regions.
- It deletes at most 500 blobs that are not referenced and have
  `last_seen_at < now - blob_gc_grace_secs`.
- A publish that races the pass has just touched its blobs, so they are inside
  the grace period. The publish transaction's existence check catches anything
  else.
- Only one pass runs at a time.

### Rollback

1. Set `blob_store: fs` (or remove the key) and restart. Nothing else changes,
   because the worktrees were never touched.
2. `registry_blobs` can then be dropped. It is recreated with `IF NOT EXISTS` if
   pg is enabled again, and the import refills it.

---

## B. Registry-driven ingest with multiple database targets (design)

### What exists today (verified in code)

- **Where ingest runs.** It is part of the sync pipeline. `MasterSyncer::sync_once`
  (`updater/sync.rs`) runs pull → unpack → prune → `ingest()` → version merge →
  git push, and the registry publishes only after `sync_once` returns. A slow
  ingest therefore delays publishing and git push, and a failed ingest re-pulls
  the whole bundle on the next trigger (`ingest_failed`).
- **Target.** In production the VM105 syncer ingests into CN08 `haruki_sekai`
  through `master_database.dsn`. That database has 116 typed tables from
  `schema_info.json`. 102 of them have the unique key `(id, server_region)`,
  where JSON `id` is mapped to `game_id`, and every table has a `server_region`
  column.
- **Write pattern.** `IngestionEngine::ingest_master_data`
  (`ingest_engine.rs`) re-ingests every `*.json` file on every run: there is no
  change detection. Each file is its own transaction:
  `DELETE FROM <table> WHERE server_region = $region`, then batched INSERTs.
  Every update therefore rewrites every row of every table of the region,
  roughly the full 417 MB of JSON as rows, which produces large WAL, bloat and
  autovacuum load. Files are committed independently, so a reader can see a
  region with some tables at the new version and others at the old one.
- **Unknown keys are dropped.** `map_target_columns` (`ingest_engine.rs`
  ~line 428) keeps only JSON keys that normalize to an existing column
  (`filter_map`) and drops the rest silently. New game fields are lost until
  the models are regenerated. `AGENTS.md`'s "unmapped keys are stored as
  json.RawMessage" describes the old Go engine, not this one.
- **Other gaps.**
  - Files with no table (`resolve_table_name` → `None`) are skipped silently.
  - An empty array leaves the old rows in place.
  - Rows of pruned tables are never deleted.
  - There is one target DSN only.

### Goals

- Ingest never blocks or fails publishing or git push.
- Only changed files are written, and each file is parsed once, however many
  targets there are.
- Each target sees a region switch versions atomically, and a failure in one
  target does not affect the others.
- No data is lost: unknown keys are kept.
- Memory stays bounded, and the design runs next to the registry on CN05 or on
  another node.

### Shape

A new role of the same binary, `master_ingest` (a `src/bin/` entry or a
`--role ingest` flag), runs as its own process and container with its own
config section. It reads nothing from the worktrees. Everything comes from the
registry over HTTP: `current`, `manifests/{hash}` and `blob/{sha256}`. The
immutable blob URLs make retries and multi-node placement trivial.

```yaml
ingest:
  registry_url: "http://127.0.0.1:9998"
  token: ""                         # registry.token, for the webhook subscription
  listen: "127.0.0.1:9997"          # receives the registry's publish webhook
  reconcile_cron: "0 */5 * * * *"   # fallback: compare every target with registry current
  parse_concurrency: 2              # files parsed at once (memory knob)
  targets:
    - name: cn08
      dsn: "postgres://.../haruki_sekai"
      regions: [jp, en, tw, kr, cn]
      tables: all                   # or an allow-list of table names
      schema: schema_info.json      # typed column map
      raw_column: true              # add/keep `raw jsonb` with the full row
      max_connections: 4
    - name: analytics
      dsn: "postgres://.../master_raw"
      tables: [cards, events, musics]
      raw_column: true
```

### Triggers

- **Webhook.** The registry's subscriber payload is extended in a
  backwards-compatible way; existing receivers ignore the unknown fields:

  ```json
  {"server":"jp","dataVersion":"...","contentHash":"<hex>","gitCommit":"<sha>|null",
   "previousContentHash":"<hex>|null",
   "changed":[{"name":"cards.json","sha256":"...","size":123}],
   "removed":["oldtable.json"]}
  ```

  The payload names a manifest, so the ingester can always re-derive it from
  `manifests/{contentHash}`. `changed` is an optimisation, not a source of
  truth. Publishing sends it and does not wait: the registry already
  notifies subscribers after the state commit and after releasing the publish
  lock.
- **Reconciliation.** On `reconcile_cron`, and at startup, the ingester reads
  each region's `current` and compares it with each target's recorded
  `content_hash`. It catches lost webhooks, ingester downtime, a newly added
  target, and a target restored from backup.

### Per-target state

The following table is created in each target database, so the state travels
with the data (backups and restores stay consistent):

```sql
master_ingest_state(region text, file text, sha256 text, table_name text,
                    rows bigint, ingested_at timestamptz, primary key(region, file))
master_ingest_version(region text primary key, content_hash text, data_version text,
                      git_commit text, started_at timestamptz, finished_at timestamptz,
                      status text, error text)
```

### Algorithm, per region

1. Fetch the target manifest.
2. For each target, compute a diff against `master_ingest_state`: changed
   files, where the sha256 differs or the file is new, and removed files. A
   target already at `content_hash` is skipped.
3. Compute the union of the changed files over all targets.
4. Handle each unioned file once:
   - Stream `blob/{sha}` (verify the sha256 while reading).
   - Parse it once in row batches (the existing `stream_rows`, about 2000 rows
     per batch).
   - Fan each batch out, through bounded channels, to the targets that need
     that file. Each target builds its own typed rows (its table set, its
     columns and `raw`).
5. Each target has **one transaction per region**. Within it:
   - Every changed file is written, with the per-table write strategy below.
   - Tables of removed files are cleared for the region (`removed`
     semantics are explicit now).
   - `master_ingest_state` and `master_ingest_version` are updated.
   - Commit.

   Readers of a target switch from one complete version to the next.
6. A slow target stalls the fan-out only through its bounded channel. If a
   target fails (connection, constraint or check), it rolls back, records
   `status=failed` and is dropped from this run. The other targets continue.
   Reconciliation retries it later.

### Write strategy per table

The goal is to avoid rewriting unchanged rows:

- Tables with a unique key: rows go into a temp table (`COPY` when available,
  otherwise batched INSERT), then
  `INSERT … ON CONFLICT (key) DO UPDATE … WHERE (t.*) IS DISTINCT FROM (excluded.*)`,
  then `DELETE … WHERE server_region = $r AND key NOT IN temp`. WAL is then
  proportional to the rows that actually changed.
- Tables without a key (2 today): DELETE and INSERT for the region, as now.
  Only changed files reach this path.

### Typed tables and unknown keys

- Typed columns keep the current mapping: normalized key, `id` → `game_id`,
  and the typed value conversions.
- With `raw_column: true`, every table has `raw jsonb`, which holds the full
  original row. No field is lost, and new game fields can be queried at once and
  promoted to typed columns later without re-downloading anything.
- Unknown keys are counted per table and reported in `master_ingest_version`
  and in the log, instead of being dropped silently.
- DDL:
  - `raw` is added with `ALTER TABLE … ADD COLUMN IF NOT EXISTS`.
  - Missing tables are created only when the target allows it (`create_tables:
    true`). Otherwise the target fails its check.
  - Typed-column changes stay manual, as today.

### Integrity checks, before a region's commit

- The number of manifest files matches the files that are handled plus the
  ones skipped because no table exists.
- Every table named in `required_tables` has at least one row for the region.
  The list defaults to the prune protect list (`BUILTIN_PROTECTED_TABLES`),
  which is already kept in sync with consumers.
- Each changed file's row count is at least `min_ratio` times its previous row
  count (0.5 by default). This guards against a truncated upstream table. A
  failed check aborts that target's transaction.

### Memory

Peak memory is `parse_concurrency` × (one blob read buffer + about 2 row
batches × number of targets). No file is ever held whole in memory, and the
compressed transport stays on the registry side.

### Rollout

1. Registry: extend the webhook payload and add `registry.subscribers` →
   ingester. This is compatible with existing receivers.
2. Run the ingester against a scratch copy of `haruki_sekai`, and compare the
   tables with the old ingest at the same `contentHash`.
3. Point the ingester at CN08 `haruki_sekai` and remove `master_database` from
   the VM105 syncer config, so the old ingest stops there. The old engine stays
   for `run_ingest` and for owner nodes until it is retired.

### Open questions for B

- **`raw` storage.** Should it be in the typed tables or in one side table
  `master_raw(region, table, key, raw)`? Keeping it in the typed tables avoids
  joins; a side table keeps the typed tables narrow.
- **COPY.** Should the target writer use sqlx's `COPY` directly (faster, and
  it bypasses SeaORM), or stay on batched INSERT?
- **Removed files.** Should the rows of a removed file be deleted right away,
  or only after two consecutive manifests without the file, matching the
  producer's prune rule?

---

## Deploy notes: part A on CN05

1. Upgrade the image and restart once with no config change (fs), as a no-op
   check.
2. Set `registry.blob_store: pg`. `state_dsn` is already set on CN05. Restart.
   - The first start creates `registry_blobs`, runs `publish_missing`, then
     imports in the background: current first, then snapshots.
   - Watch the log line `Blob import done in …s: N stored (in -> out bytes), …
     unavailable`.
   - Reads are served from disk until the import finishes.
3. Check: `SELECT count(*), pg_size_pretty(sum(stored_size)), pg_size_pretty(sum(size)) FROM registry_blobs;`
   Also check that `files/`, `blob/` and `bundle` responses are unchanged,
   comparing ETag and body with `blob_store: fs`.
4. Rollback: see above. The worktrees and git are unaffected throughout.

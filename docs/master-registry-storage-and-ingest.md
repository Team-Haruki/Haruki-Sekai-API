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
| `registry.blob_gc_grace_secs` | `86400` | an unreferenced blob is deleted only after no publish has referenced it for this long (minimum 300; smaller values are raised with a warning) |

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
   otherwise. On PostgreSQL the check reads the rows `FOR KEY SHARE`, which
   conflicts with GC's `DELETE`: a GC delete that is already running makes the
   check see the row as missing (the publish fails and the next one re-stores
   it), and a later one waits until the publish commits. After that it switches `current`, writes the snapshot, prunes the
   snapshots and appends history. As a result, `current` never lists a blob that
   cannot be served.
4. Subscribers are notified, then one GC pass runs.

### Reads

| endpoint | fs (unchanged) | pg |
|---|---|---|
| `current`, `manifests/{hash}`, `history` | state | state |
| `files/{name}` | the file on disk (whatever is there now) | resolved through the current manifest, then the blob (see the edge case below) |
| `blob/{sha256}` | only files of the current manifest whose bytes on disk still hash to the digest | any stored blob: current, or listed by a retained snapshot of any region, or not yet collected |
| `bundle` | tar of the directory (readdir order, file metadata); 404 when it holds no `*.json` | tar built from the blobs of `current` (manifest order, mode 0644, mtime = publish time) plus a version entry built from the manifest; 404 when `current` lists no files |

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
  while any blob is missing, and whenever the store fails before the response
  starts.
- **Slow or unreachable database.** One read gets 2 s (`QUERY_TIMEOUT`) for the
  read slot, the query slot and the query together; the pool's own 30 s
  acquire timeout is never reached. A failed or timed-out query opens a 5 s
  circuit breaker (`BREAKER_OPEN`): reads then skip the database, so current
  `files/` and `blob/` requests go straight to disk.
- **Historical blob while the store cannot answer** (database down, breaker
  open, no read slot): `503` with `Cache-Control: no-store` and
  `Retry-After: 5`, not a 500. A `404` on a digest URL (`blob/`,
  `manifests/`, music_metas `blob/`), for example a digest whose import has
  not run yet, also carries `no-store`, so a CDN never pins a miss on an
  otherwise immutable URL.
- **`files/` edge case.** With pg, `files/{name}` resolves through `current`.
  While a file's blob is not stored yet (import running) and its bytes on
  disk no longer match the manifest digest, the fallback's digest check
  fails and the answer is a 404 (fs would serve the new bytes on disk). The
  window closes once the import stores the blob (from git if needed) or the
  next publish lists the new bytes.
- **Bundle never breaks off.** Before answering, the bundle loads the
  compressed blobs of every file `current` lists (about 8 MB for a full
  region, at most `BUNDLE_CONCURRENCY` = 4 bundles at once, 20 s budget).
  Only then is the 200 sent; the tar is decoded from memory by a blocking
  task into a bounded channel of eight 64 KiB chunks, so a database error can
  no longer truncate a response mid-stream. Any failure before that falls
  back to the directory tar.
- **Memory.** At most 16 blob responses are in flight at once
  (`READ_CONCURRENCY`, a semaphore held for the whole stream). The 37 MB file
  stores as 0.85 MB, so the largest blob is about 1.3 MB, and each decoder has
  a 1 MiB window: about 40 MB worst case for blobs, plus about 4 × 8 MB for
  bundles being sent. The state pool is 8 connections with `blob_store: pg`
  (4 with fs, as before). At most 4 blob row queries run at once, so state
  writes always find a connection. A blob holds a connection only for its
  fetch, not while the body streams. No extra server-side cache is added: blob
  URLs are immutable and EdgeOne caches them, `files/` is revalidated with
  304s, and decompressing 1.5 MB takes about 2 ms.

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
    concurrent full bundles and 8 concurrent 37 MB blob downloads afterwards
    (measured before bundles loaded their blobs up front; that adds about
    8 MB per bundle being sent, at most 4 at once).
  - One full bundle streamed in 0.5 s.

Reads keep working during the import through the directory fallback.

### Garbage collection

After every changed publish, and after the import, one pass runs:

- It computes the referenced set: every digest in the current manifest and the
  retained snapshots of all regions.
- It deletes at most 500 blobs that are not referenced and have
  `last_seen_at < now - blob_gc_grace_secs`.
- A publish that races the pass has just touched its blobs, so they are inside
  the grace period (at least 300 s). The publish transaction's locked
  existence check catches anything else (see the publish flow).
- If any current manifest or retained snapshot of any region cannot be read
  or parsed, the pass is aborted with a warning and deletes nothing: an
  unreadable snapshot must not make its blobs look unreferenced.
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
  `schema_info.json`, and every table has a `server_region` column. 102 of
  them have the unique key `(id, server_region)`, where JSON `id` is mapped to
  `game_id`; 12 have another composite key (for example
  `(card_rarity_type, server_region)` or `(event_id, music_id, server_region)`);
  2 have no key at all (`ngwords`, `resourceboxdetails`).
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

**The ingester requires `registry.blob_store: pg`.** With the fs store,
`blob/{sha256}` serves only files of the *current* manifest, so every blob of a
manifest the ingester is still working on turns into a 404 as soon as the
registry publishes the next version. The ingester checks this at startup (the
registry's `/health` reports `blobStore`) and refuses to run against an fs
registry.

```yaml
ingest:
  registry_url: "http://127.0.0.1:9998"
  listen: "127.0.0.1:9997"          # receives the registry's publish webhook
  webhook_token: ""                 # required bearer token on the webhook (empty = webhook off)
  reconcile_cron: "0 */5 * * * *"   # fallback: compare every target with registry current
  parse_concurrency: 2              # files parsed at once (memory knob)
  targets:
    - name: cn08                    # the key of this target's state rows; never reuse or rename
      dsn: "postgres://.../haruki_sekai"
      regions: [jp, en, tw, kr, cn]
      tables: all                   # or an allow-list of table names
      schema: schema_info.json      # typed column map
      raw: true                     # keep the full row in master_raw (side table)
      required_tables: default      # default = the prune protect list
      min_ratio: 0.5
      max_connections: 4
    - name: analytics
      dsn: "postgres://.../master_raw"
      tables: [cards, events, musics]
      raw: true
```

### Triggers

- **Webhook.** The registry's subscriber payload is extended in a
  backwards-compatible way; existing receivers ignore the unknown fields:

  ```json
  {"server":"jp","dataVersion":"...","contentHash":"<hex>","gitCommit":"<sha>|null"}
  ```

  The webhook is a *trigger only*. The ingester never ingests the hash in the
  payload; it starts a reconcile of that region, which reads the registry's
  `current` at that moment. Two webhooks in quick succession, or one that
  arrives late, therefore cannot make a target ingest an older version.
  Publishing sends it and does not wait: the registry already notifies
  subscribers after the state commit and after releasing the publish lock.
- **Webhook auth.** The endpoint requires `Authorization: Bearer
  <ingest.webhook_token>` (the registry's `subscribers[].token`, which it
  already sends). A missing or wrong token is a 401; an empty
  `webhook_token` disables the endpoint (404), leaving only reconciliation.
  Even an authenticated webhook can only cause a reconcile, never choose the
  data.
- **Reconciliation.** On `reconcile_cron`, at startup and on every webhook,
  the ingester reads each region's `current` and compares it with each
  target's recorded version. It catches lost webhooks, ingester downtime, a
  newly added target, and a target restored from backup.

### Concurrency and ordering

- One reconcile per `(target, region)` at a time: a PostgreSQL advisory lock
  on the target database, keyed by `hashtext('master_ingest:' || target || ':'
  || region)`, taken with `pg_try_advisory_lock` for the whole run (a second
  ingester process against the same target skips the region instead of
  interleaving). In-process, a per-`(target, region)` mutex coalesces
  triggers: a trigger that arrives during a run schedules exactly one follow-up
  run.
- **Never backwards.** A target's version only moves forward. Before a run
  commits, it re-reads `master_ingest_version` under the advisory lock and
  aborts if the recorded `data_version` is newer than the one it is ingesting
  (versions compared with the existing `compare_version`). An equal
  `data_version` with a different `content_hash` (a re-publish of the same
  version) is allowed.
- **Pruned manifest.** The registry keeps 20 snapshots per region. If
  `manifests/{hash}` or one of its blobs answers 404 while a run is in
  progress (the registry moved on and pruned it), the run is abandoned without
  commit and restarted from the new `current`. A 503 is retried with backoff.

### Per-target state

The following tables are created in each target database, so the state
travels with the data (backups and restores stay consistent). Every row is
keyed by the **target name** as well, so two targets that point at the same
database (for example a full target and an allow-list target sharing one
instance) never read or overwrite each other's state:

```sql
master_ingest_state(target text, region text, file text, sha256 text, table_name text,
                    mapping_hash text, rows bigint, missing_since text,
                    ingested_at timestamptz, primary key(target, region, file))
master_ingest_version(target text, region text, content_hash text, data_version text,
                      git_commit text, started_at timestamptz, finished_at timestamptz,
                      status text, error text, primary key(target, region))
master_raw(region text, table_name text, key jsonb, raw jsonb,
           primary key(region, table_name, key))
```

### Mapping hash

A file's `sha256` alone is not enough to skip it: the same bytes produce
different rows when the mapping changes. Each `(target, table)` has a
**mapping hash**, SHA-256 over:

- the table's entry in `schema_info.json` (columns, types, unique keys),
- the ingest engine version (a constant bumped whenever the value conversion
  or key normalization changes),
- the target's `raw` setting,
- the target's resolved table set.

It is stored per file in `master_ingest_state.mapping_hash`. A file is
re-ingested when its `sha256` **or** its mapping hash differs, so regenerating
the models, upgrading the engine or turning `raw` on rewrites exactly the
affected tables even though no master file changed.

### Algorithm, per region

1. Take the advisory lock, read the registry's `current` and fetch
   `manifests/{contentHash}`.
2. For each target, diff against `master_ingest_state`: changed files (new
   file, different `sha256` or different mapping hash) and files no longer
   listed. A target already at `content_hash` with no mapping change is
   skipped.
3. Compute the union of the changed files over all targets.
4. Handle each unioned file once:
   - Stream `blob/{sha}` (verify the sha256 while reading).
   - Parse it once in row batches (the existing `stream_rows`, about 2000 rows
     per batch).
   - Fan each batch out, through bounded channels, to the targets that need
     that file. Each target builds its own typed rows (its table set, its
     columns) and, with `raw`, its `master_raw` rows.
5. Each target applies the region in **one transaction** in steady state
   (a master update touches a few dozen tables). Within it:
   - Every changed file is written, with the per-table write strategy below.
   - Removed files are handled with the prune rule below.
   - The integrity checks run.
   - `master_ingest_state` and `master_ingest_version` are updated.
   - Commit. Readers of a target switch from one complete version to the next.
6. A slow target stalls the fan-out only through its bounded channel. If a
   target fails (connection, constraint or check), it rolls back, records
   `status=failed` and is dropped from this run. The other targets continue.
   Reconciliation retries it later.

### First full ingest

A new target, or a mapping change that touches most tables, would otherwise
rewrite the full ~417 MB in one transaction per region (huge WAL, long locks,
one failure redoes everything). Instead, when a run would rewrite more than a
threshold (default: 25 tables or 50 MB of JSON), it is **staged per table**:

- Each table is loaded into a staging table (`<table>__ingest`), checked, and
  swapped in its own short transaction (upsert/delete from staging, or
  `ALTER TABLE … RENAME` for a table that is empty for the other regions),
  recording that file's `master_ingest_state` row in the same transaction.
- `master_ingest_version.status` stays `staging` until every table is done,
  then the final transaction sets the version. Readers see a mix of old and
  new tables during a first load; that is acceptable once and is reported by
  `status`.
- A failure resumes where it stopped: finished tables already carry the new
  `sha256` and mapping hash.

### Write strategy per table

The goal is to avoid rewriting unchanged rows:

- Tables with a unique key (114 of 116: `(id, server_region)` or another
  composite key): rows are written with **batched multi-row `INSERT … ON
  CONFLICT (key) DO UPDATE … WHERE (t.*) IS DISTINCT FROM (excluded.*)`**, then
  `DELETE … WHERE server_region = $r AND key NOT IN (keys of this file)`. WAL is
  then proportional to the rows that actually changed. `COPY` is not used:
  batched INSERT stays on SeaORM/sqlx without a second write path and is fast
  enough at a few dozen changed tables per update.
- Tables without a key (2 today: `ngwords`, `resourceboxdetails`): DELETE and
  INSERT for the region, as now. Only changed files reach this path.

### Typed tables and unknown keys

- Typed columns keep the current mapping: normalized key, `id` → `game_id`,
  and the typed value conversions.
- With `raw: true`, the full original row goes into the **side table
  `master_raw(region, table_name, key, raw)`**, not into a column of the typed
  tables. The typed tables stay narrow and unchanged for existing readers; no
  field is lost, and new game fields can be queried at once and promoted to
  typed columns later without re-downloading anything. Rows of keyless tables
  use their row index within the file as `key`.
- Unknown keys are counted per table and reported in `master_ingest_version`
  and in the log, instead of being dropped silently.
- DDL:
  - The state tables and `master_raw` are created with `IF NOT EXISTS`.
  - Missing typed tables are created only when the target allows it
    (`create_tables: true`). Otherwise the target fails its check.
  - Typed-column changes stay manual, as today.

### Removed tables

A file that disappears from the manifest is handled with the producer's prune
rule (`updater/prune.rs`), not immediately:

- The first manifest without the file records `missing_since` in its
  `master_ingest_state` row and keeps the rows.
- The table's rows for the region are deleted only when a **second
  consecutive** ingested version still lacks the file. A file that comes back
  in between clears `missing_since`.
- Tables on the protect list (`BUILTIN_PROTECTED_TABLES`) and tables in the
  target's `required_tables` are never cleared this way; a manifest that drops
  one of them fails the `required_tables` check below instead, so the
  operator sees it.

### Integrity checks, before a region's commit

- The number of manifest files matches the files that are handled plus the
  ones skipped because no table exists.
- Every table named in `required_tables` has at least one row for the region
  after the write. The list defaults to the prune protect list
  (`BUILTIN_PROTECTED_TABLES`), which is already kept in sync with consumers.
- Each changed file's row count is at least `min_ratio` times its previous row
  count (0.5 by default). This guards against a truncated upstream table. A
  failed check aborts that target's transaction.
- **Manual override for `min_ratio`.** A legitimate large shrink (the game
  really removed most rows of a table) would otherwise fail forever. The
  operator allows it for one specific version: `POST
  /v1/ingest/{target}/{region}/allow-shrink` with `{"contentHash": "...",
  "tables": ["..."]}` (same bearer token), or the same list in the config
  (`allow_shrink: [{content_hash, tables}]`). The override applies only to that
  `contentHash` and is recorded in `master_ingest_version.error` for the
  audit trail; the next version is checked normally.

### Memory

Peak memory is `parse_concurrency` × (one blob read buffer + about 2 row
batches × number of targets). No file is ever held whole in memory, and the
compressed transport stays on the registry side.

### Rollout

1. Registry: extend the webhook payload and add `registry.subscribers` →
   ingester (with its token). This is compatible with existing receivers. The
   registry must run `blob_store: pg`.
2. Run the ingester against a scratch copy of `haruki_sekai`, and compare the
   tables with the old ingest at the same `contentHash`.
3. Point the ingester at CN08 `haruki_sekai` and remove `master_database` from
   the VM105 syncer config, so the old ingest stops there. The old engine stays
   for `run_ingest` and for owner nodes until it is retired.

### Decisions (defaults)

These were open questions; the operator decided them as follows:

- **`raw` storage:** a side table `master_raw(region, table_name, key, raw)`,
  not a column in the typed tables.
- **Writes:** batched `INSERT … ON CONFLICT DO UPDATE` (upsert), not `COPY`.
- **Removed tables:** rows are deleted only after two consecutive ingested
  versions without the file (the producer's prune rule), never for protected
  or required tables.
- **Historical blob while PostgreSQL is down:** the registry answers `503`
  with `Cache-Control: no-store` (implemented in part A); the ingester retries.
- **No zstd `Content-Encoding` on blob responses:** bodies stay byte-identical
  to the fs store, and a CDN holds one variant per URL.

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

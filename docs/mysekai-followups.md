# MySekai master follow-ups

Adds `mysekaiTools`, `mysekaiSites`, and `mysekaiCharacterTalkPreActions` to
typed master ingestion. All three files already appear in the five-region
manifest fixture; this does not change any regional app identity or Nuverse
schema bundle.

## Source and regeneration

Models come from `Sekai.ApiData.MasterMysekaiTool`, `MasterMysekaiSite`, and
`MasterMysekaiCharacterTalkPreAction` in the JP 7.0.0 iOS `dump/dump.cs` from
the `il2cpp_analysis.zip` asset of `middlered/sekay` release `v7.0.0_ios`.
The small test fixtures are actual JP 7.0.0.13 master rows. The free timeline
group ID test is synthetic: the client declares the field, but those current
master rows omit it.

```sh
python3 tools/generate_mysekai_followups.py /path/to/dump.cs
cd tools/ent_generator
cargo run --locked
cd ../..
cp schema_info_generated.json schema_info.json
cargo fmt --all
```

`coolTimeMicroSeconds` is a client float, stored as `double precision` rather
than an integer. Fields are optional, including `presetGroupId` and the talk
action references. The tables use `(game_id, server_region)` unique keys.

`MasterMysekaiCharacterTalkFreeTimeline` itself is not part of `SuiteMaster`
and has no corresponding file in the current manifest fixture. Do not add
a typed table until there is a verified master source for it. The nullable
`mysekai_character_talk_free_timeline_group_id` reference is ready for data.

## Rollout

1. Check the live master database and owner role on CN08. Existing master
   tables should be owned by `haruki_sekai`; inspect any pre-existing tables
   with these three names before applying the migration.
2. Run `docs/migrations/2026-09-30-mysekai-followups.sql` with
   `psql -v ON_ERROR_STOP=1`. It uses `SET ROLE haruki_sekai`, creates the
   tables and unique indexes in a transaction, and can be rerun.
3. Deploy the updated `master_ingest` and let its schema reconciliation run.
   Deploying first can abort region preparation because production does not
   automatically create missing typed tables.
4. Confirm all three tables have rows for the expected regions and verify JP
   tool IDs 5 and 10 expose their names and `pickax0005` / `ax0005` icon fields.
   Check ingester logs for mapping or missing-table errors.

Cloud still needs its own file-to-table mappings and `mysekai_tool` display
handling. These tables supply metadata; actual asset paths must be verified
against the exported assets. No Cloud, Drawing, or Toolbox changes are included.

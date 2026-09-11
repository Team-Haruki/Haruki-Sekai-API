> [!Caution]
> This project was rewritten in Rust.  
> Go edition and Python edition are not maintained anymore.  
> If you want to use Python edition, please go to [old python branch](https://github.com/Team-Haruki/Haruki-Sekai-API/tree/old-python).  
> If you want to use Go edition, please go to [old go branch](https://github.com/Team-Haruki/Haruki-Sekai-API/tree/old-go).

# Haruki Sekai API

**Haruki Sekai API** is a companion project for [HarukiBot](https://github.com/Team-Haruki), providing direct API access to various servers of the game `Project Sekai: Colorful Stage`.

## Requirements
+ `MySQL`, `SQLite`, `PostgreSQL` (Optional, depending on your database choice)
+ `Redis` (Optional, for caching sekai users)

## How to Use
1. Go to release page to download `haruki-sekai-api`
2. Rename `haruki-sekai-configs.example.yaml` to `haruki-sekai-configs.yaml` and then edit it.
3. Make a new directory or use an exists directory
4. Put `haruki-sekai-api` and `haruki-sekai-configs.yaml` in the same directory
5. Edit `haruki-sekai-configs.yaml` and configure it
6. Open Terminal, and `cd` to the directory
7. Run `haruki-sekai-api`

## Master Data Registry

`master_registry` (shipped next to `haruki-sekai-api`) is the master data manager: it pulls each region's master from its owner node (`servers.<region>.master_sync.source_url`), owns git push and database ingest, publishes per-region manifests, maintains the `music_metas` feed and serves everything other projects consume.

1. Reuse `haruki-sekai-configs.yaml`; fill in the `registry:` section (`token`, `state_dir`, `subscribers`, `music_metas`).
2. Run `master_registry` from the same directory (`CONFIG_PATH` is honoured like the API server).
3. Consumers read `GET /v1/master/{region}/current` (revalidate with `If-None-Match`), then fetch changed files by digest from `GET /v1/master/{region}/blob/{sha256}` (immutable). `GET /v1/metas/{region}/current` and `blob/{sha256}` work the same way for music metas; `GET /v1/app/{region}` serves the app identity in the `apphash_sources` `url` shape.
4. Point an owner's `master_sync.notify` at `POST /internal/master-updated` on the registry; mutating endpoints require `Authorization: Bearer <registry.token>`.

## Nuverse Schema

See [docs/nuverse-schema-guide.md](docs/nuverse-schema-guide.md).

## License

This project is licensed under the MIT License.

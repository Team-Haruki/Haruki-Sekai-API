# Nuverse Schema

Haruki Sekai API restores Nuverse compact msgpack payloads with committed schema assets.

Runtime files live in [`Data/structures`](../Data/structures):

- `nuverse_schema_bundle.json`: runtime bundle used by [`src/client/nuverse_schema.rs`](../src/client/nuverse_schema.rs)
- `master.avsc`: generated `Sekai.Master*` schemas
- `suite.avsc`: generated `Sekai.SuiteUser` and `Sekai.User*` schemas

For TW/KR/CN, point `nuverse_schema_bundle_path` at:

```yaml
nuverse_schema_bundle_path: "Data/structures/nuverse_schema_bundle.json"
```

The bundle drives both:

- Nuverse master restore from `master-data-<cdnVersion>.info`
- Nuverse API response restore for mapped profile and ranking fields

## Source Of Truth

Schema generation, AVSC format details, and Go/Python/Rust consumption examples are maintained in StructTool:

[Team-Haruki/Haruki-Nuverse-StructTool](https://github.com/Team-Haruki/Haruki-Nuverse-StructTool)

Use StructTool `main` for the current generator and parser examples. This repository only keeps the generated runtime assets and the Rust restore integration.

## Field Naming

Haruki's committed schemas use JSON output field names, not raw C# member names. During generation, field names are normalized to camelCase and leading backing underscores are removed while `msgpack_key` preserves the original compact-msgpack key.

Examples:

- `Id` with `msgpack_key: 0` becomes field name `id`
- `ExchangeCategory` with `msgpack_key: 2` becomes field name `exchangeCategory`
- `_assetbundleName` with `msgpack_key: 11` becomes field name `assetbundleName`

Do not replace these assets with raw exporter output unless the same normalization has been applied, or restored JSON can contain PascalCase keys or duplicate PascalCase/camelCase fields.

## Updating Assets

Regenerate the schemas from the CN DummyDll source, then copy the generated assets back into this repository:

```text
~/Desktop/pjskida/cn/DummyDll
```

Commit all three generated files together:

- `Data/structures/nuverse_schema_bundle.json`
- `Data/structures/master.avsc`
- `Data/structures/suite.avsc`

After updating, run:

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

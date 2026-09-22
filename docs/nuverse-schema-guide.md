# Nuverse Schema

Haruki Sekai API restores Nuverse compact msgpack payloads with committed schema assets.

Versioned asset sets live in `Data/structures/6.0.0/` and `Data/structures/6.4.0/`. Each contains the runtime bundle, both AVSC files, and a SHA-256 manifest. The root-level files remain unchanged as legacy 6.0.0 compatibility copies. The 6.4.0 set comes from CN iOS metadata and is also used by the verified TW deployment; KR stays on 6.0.0 until its rollout is validated.

Runtime files live in [`Data/structures`](../Data/structures):

- `nuverse_schema_bundle.json`: runtime bundle used by [`src/client/nuverse_schema.rs`](../src/client/nuverse_schema.rs)
- `master.avsc`: generated `Sekai.Master*` schemas
- `suite.avsc`: generated `Sekai.SuiteUser` and `Sekai.User*` schemas

For CN/TW 6.4.0, point `nuverse_schema_bundle_path` at:

```yaml
nuverse_schema_bundle_path: "Data/structures/6.4.0/nuverse_schema_bundle.json"
```

The bundle drives both:

- Nuverse master restore from `master-data-<cdnVersion>.info`
- Nuverse API response restore for mapped profile and ranking fields

## Source Of Truth

The runtime bundle has an in-repo generator: [`tools/nuverse_schema_generator`](../tools/nuverse_schema_generator) (C#, Mono.Cecil). It reads `Assembly-CSharp.dll` from an Il2Cpp DummyDll dump and writes `Data/structures/nuverse_schema_bundle.json`:

```bash
cd tools/nuverse_schema_generator
dotnet run -- /path/to/DummyDll ../../Data/structures/nuverse_schema_bundle.json
```

AVSC format details and Go/Python/Rust consumption examples are maintained in StructTool:

[Team-Haruki/Haruki-Nuverse-StructTool](https://github.com/Team-Haruki/Haruki-Nuverse-StructTool)

Use StructTool `main` for the AVSC generator and parser examples. Besides the Rust restore integration, this repository keeps the generated runtime assets and the bundle generator above.

## Field Naming

Haruki's committed schemas use JSON output field names, not raw C# member names. During generation, field names are normalized to camelCase and leading backing underscores are removed while `msgpack_key` preserves the original compact-msgpack key.

Examples:

- `Id` with `msgpack_key: 0` becomes field name `id`
- `ExchangeCategory` with `msgpack_key: 2` becomes field name `exchangeCategory`
- `_assetbundleName` with `msgpack_key: 11` becomes field name `assetbundleName`

Do not replace these assets with raw exporter output unless the same normalization has been applied, or restored JSON can contain PascalCase keys or duplicate PascalCase/camelCase fields.

## Nuverse 6.4 login compatibility

CN and TW 6.4 return account/session information from `POST /api/user/auth`;
master versions come from `POST /api/user/{userId}/login`. The master updater
and internal login probe fetch that metadata when a Nuverse auth response lacks
data, asset, or CDN versions. Complete legacy responses and CP logins keep their
existing flow. This also covers KR's planned 6.4 upgrade without switching its
live credentials before the rollout.

On 2026-09-22, TW 6.4 login was verified with the CN 6.4 AES key/IV and appHash,
`appVersion: 6.4.0`, and the `device_id` request header from the CN configuration.
Without that header, auth returned an encrypted `403 session_error`. Keep these
values in the private deployment configuration. Revalidate them against KR after
its upgrade; sharing the protocol does not establish that its credentials match.

TW CDN 274 (data 6.4.0.2) still used its pre-6.4 AES key/IV, so it required
`master_aes_key_hex` and `master_aes_iv_hex` separately from the new API cipher.
Later on 2026-09-22, CDN 275 (data 6.4.0.3) switched to the new API cipher;
clear both master overrides for that payload. Both override fields must be set
together; leaving both empty uses the API cipher. Verify the CDN cipher separately
during KR's upgrade, even if its API accepts the same identity as CN/TW.

Keep region-specific bundles in separate directories when regions upgrade at
different times. CN 6.4 assets are in `Data/structures/6.4.0/`; switching a region's
`nuverse_schema_bundle_path` requires restarting the account service.

## Updating Assets

Regenerate the schemas from the CN DummyDll source, then copy the generated assets back into this repository:

```text
~/Desktop/pjskida/cn/DummyDll
```

Commit all three generated files together:

- `Data/structures/<game-version>/nuverse_schema_bundle.json`
- `Data/structures/<game-version>/master.avsc`
- `Data/structures/<game-version>/suite.avsc`

After updating, run:

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

# Nuverse Schema Guide

This document describes where the Nuverse master and suite schemas come from, how to generate them, and how to use them to restore compact msgpack payloads in Rust or Python.

## Overview

Nuverse regions use compact msgpack layouts in two places:

- `master-data-<cdnVersion>.info` payloads from the Nuverse CDN
- suite or API payloads that embed `Sekai.User*` records as flat arrays

The schema bundle committed in this repository is generated from DummyDll metadata and follows the custom Avro format used by [unity-msgpack-schema-exporter](https://github.com/middlered/unity-msgpack-schema-exporter).

In this repository:

- [Data/structures/nuverse_schema_bundle.json](/Users/seiun/RustroverProjects/Haruki-Sekai-API/Data/structures/nuverse_schema_bundle.json) is the runtime bundle used by Rust restore code
- [Data/structures/master.avsc](/Users/seiun/RustroverProjects/Haruki-Sekai-API/Data/structures/master.avsc) contains the generated `Sekai.Master*` schema set
- [Data/structures/suite.avsc](/Users/seiun/RustroverProjects/Haruki-Sekai-API/Data/structures/suite.avsc) contains the generated `Sekai.SuiteUser` and `Sekai.User*` schema set
- [tools/nuverse_schema_generator/Program.cs](/Users/seiun/RustroverProjects/Haruki-Sekai-API/tools/nuverse_schema_generator/Program.cs) generates the bundle from DummyDll
- [src/client/nuverse_schema.rs](/Users/seiun/RustroverProjects/Haruki-Sekai-API/src/client/nuverse_schema.rs) contains the Rust schema-driven restore implementation

## Data Sources

### Master data

Nuverse master data comes from the region CDN endpoint:

```text
{nuverse_master_data_url}/master-data-{cdnVersion}.info
```

Examples from the local config:

- TW: `https://lf16-mkovscdn-sg.bytedgame.com/obj/sf-game-alisg/gdl_app_5245/MasterData/60001`
- KR: `https://lf19-mkkr.bytedgame.com/obj/sf-game-alisg/gdl_app_292248/MasterData/60001`
- CN: `https://lf9-mkcncdn-tos.dailygn.com/obj/sf-game-lf/gdl_app_5236/MasterData/60001`

The `cdnVersion` is normally taken from the Nuverse login response and stored in the region version file.

### Suite and API data

Suite or API restore targets come from `Sekai.User*` definitions in DummyDll.

Typical examples:

- profile response: `userHonors[]`, `userProfileHonors[]`
- ranking responses: `rankings[].userCard`, `borderRankings[].userCard`

These are mapped in the bundle `api` section as field selectors.

## Generating the schema bundle

### Inputs

- DummyDll folder containing `Assembly-CSharp.dll`
- Runtime metadata for `MessagePackObject`, `Key`, nullable annotations, arrays, maps, and nested records

For this repository, the current input path is:

```text
~/Desktop/pjskida/cn/DummyDll
```

### Generator command

From the repository root:

```bash
dotnet run --project tools/nuverse_schema_generator -- ~/Desktop/pjskida/cn/DummyDll Data/structures/nuverse_schema_bundle.json
```

What the generator does:

- scans `Sekai.Master*` and `Sekai.User*` records from `Assembly-CSharp.dll`
- keeps only `MessagePackObject` types and `Key`-annotated fields
- emits custom-Avro-style record definitions with `msgpack_key`
- generates `master` mappings such as `actionSets -> Sekai.MasterActionSet`
- generates `api` field mappings for common profile and ranking restore paths

The repository also keeps split Avro exports for external tooling:

- `Data/structures/master.avsc`
- `Data/structures/suite.avsc`

### Current runtime behavior

For Nuverse regions, the default bundle path is:

```text
Data/structures/nuverse_schema_bundle.json
```

See [src/config.rs](/Users/seiun/RustroverProjects/Haruki-Sekai-API/src/config.rs).

## Bundle format

The bundle has three sections:

```json
{
  "schemas": [],
  "master": {},
  "api": []
}
```

- `schemas`: Avro-like named schema definitions
- `master`: top-level master key to item schema name
- `api`: path-based whole-response or field-selector restore mappings

See the minimal example in [Data/structures/nuverse_schema_bundle.example.json](/Users/seiun/RustroverProjects/Haruki-Sekai-API/Data/structures/nuverse_schema_bundle.example.json).

## Rust restore example

This repository already contains the full Rust implementation in [src/client/nuverse_schema.rs](/Users/seiun/RustroverProjects/Haruki-Sekai-API/src/client/nuverse_schema.rs).

Minimal example for master payload restore:

```rust
use std::fs;

use haruki_sekai_api::client::nuverse_schema::NuverseSchemaStore;
use haruki_sekai_api::crypto::SekaiCryptor;

fn main() -> anyhow::Result<()> {
    let bundle = fs::read("Data/structures/nuverse_schema_bundle.json")?;
    let store = NuverseSchemaStore::from_slice(&bundle)?;

    let cryptor = SekaiCryptor::from_hex(
        "SEKAI_AES_KEY_HEX",
        "SEKAI_AES_IV_HEX",
    )?;

    let encrypted = fs::read("/tmp/cn-master-data-149.info")?;
    let msgpack = cryptor.decrypt_msgpack(&encrypted)?;
    let restored = store.restore_master_msgpack(&msgpack)?;

    println!("{}", serde_json::to_string_pretty(&restored)?);
    Ok(())
}
```

Minimal example for API field restore:

```rust
use std::fs;

use haruki_sekai_api::client::nuverse_schema::NuverseSchemaStore;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let bundle = fs::read("Data/structures/nuverse_schema_bundle.json")?;
    let store = NuverseSchemaStore::from_slice(&bundle)?;

    let body = json!({
        "rankings": [
            { "userCard": [100, 30, 1, 2, 3, 4, 5, 0, "done", "normal", 0, 1711000000, []] }
        ]
    });

    let restored = store.restore_api_json(
        "/user/{userId}/event/123/ranking?rankingViewType=top100",
        body,
    )?;

    println!("{}", serde_json::to_string_pretty(&restored)?);
    Ok(())
}
```

## Python restore example

The Python example below shows the same bundle shape and restore rules for record, array, map, union, and int-keyed fields.

```python
import json
from pathlib import Path

import msgpack


def load_registry(bundle_path: str):
    bundle = json.loads(Path(bundle_path).read_text())
    registry = {}
    for schema in bundle["schemas"]:
        if isinstance(schema, dict) and schema.get("type") == "record":
            name = schema.get("name", "")
            namespace = schema.get("namespace", "")
            full_name = f"{namespace}.{name}" if namespace and "." not in name else name
            if full_name:
                registry[full_name] = schema
            if name:
                registry[name] = schema
    return bundle, registry


def resolve_schema(schema, registry):
    if isinstance(schema, str):
        if schema in {"null", "boolean", "int", "long", "float", "double", "bytes", "string"}:
            return schema
        return registry.get(schema, schema)
    return schema


def restore_value(schema, value, registry):
    schema = resolve_schema(schema, registry)
    if isinstance(schema, str):
        return value
    schema_type = schema.get("type")
    if isinstance(schema_type, list):
        non_null = next((item for item in schema_type if item != "null"), "null")
        return None if value is None else restore_value(non_null, value, registry)
    if schema_type == "array":
        return [restore_value(schema["items"], item, registry) for item in value]
    if schema_type == "map":
        return {k: restore_value(schema["values"], v, registry) for k, v in value.items()}
    if schema_type == "record":
        if isinstance(value, list):
            by_key = {}
            for field in schema.get("fields", []):
                msgpack_key = field.get("msgpack_key", field["name"])
                if isinstance(msgpack_key, int) and msgpack_key < len(value) and value[msgpack_key] is not None:
                    by_key[field["name"]] = restore_value(field["type"], value[msgpack_key], registry)
            return by_key
        if isinstance(value, dict):
            out = dict(value)
            for field in schema.get("fields", []):
                msgpack_key = field.get("msgpack_key", field["name"])
                raw = value.get(str(msgpack_key)) if isinstance(msgpack_key, int) else value.get(msgpack_key)
                if raw is not None:
                    out[field["name"]] = restore_value(field["type"], raw, registry)
            return out
    return value


bundle, registry = load_registry("Data/structures/nuverse_schema_bundle.json")
master_map = bundle["master"]

msgpack_bytes = Path("/tmp/cn-master-data-149.info.msgpack").read_bytes()
payload = msgpack.unpackb(msgpack_bytes, raw=False, strict_map_key=False)

restored = {}
for key, value in payload.items():
    schema_name = master_map.get(key)
    restored[key] = [restore_value(schema_name, item, registry) for item in value] if schema_name and isinstance(value, list) else value

print(json.dumps(restored, ensure_ascii=False, indent=2))
```

Notes:

- If your input is still AES-encrypted, decrypt it before calling `msgpack.unpackb`.
- The Python example is intentionally minimal and mirrors the Rust logic at a high level.
- For production use in this repository, prefer the Rust implementation.

## StructTool reference repo

The current StructTool v2 repository was cloned locally for reference:

```text
/tmp/Haruki-Nuverse-StructTool-v2
```

Its CLI usage is:

```bash
go run . --schema <schema.avsc.json> --class <ClassName> --hex <hex>
```

## Validation

Recommended checks after regenerating or updating the bundle:

```bash
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
dotnet build tools/nuverse_schema_generator
```

# Compact JP catalog exports

The master syncer additionally exports `compactCards.json` and
`compactGachas.json` for JP. The original `cards.json` and `gachas.json` retain
their bytes and schema. Both variants are listed in the registry manifest,
published as immutable blobs, and included in the git mirror.

The compact files remain arrays of catalog records. Only `cardParameters` in
cards and `gachaDetails` in gachas are transformed: a nonempty array of objects
with identical field sets becomes a column-oriented object when that is smaller.
All fields, IDs, row order, nulls, booleans, numbers and strings are retained.
Heterogeneous rows, empty arrays and native object values are preserved as-is.
The entire export is JSON without indentation.

Column objects follow the Nuverse `__ENUM__` convention. Each ordinary property
is an equal-length array; row `i` consists of the value at index `i` in every
column. `__ENUM__` maps dictionary-encoded column names to their value arrays:
the column's integers are zero-based indices into that dictionary. Explicit
null cells remain null. An empty `__ENUM__` is retained as an encoding marker,
so readers can distinguish generated columns from unmodified native objects.
Dictionary encoding is used only when it saves bytes. For example:

```json
{"id":[1,2],"cardId":[2,3],"weight":[1,1],"isWish":[false,false],"__ENUM__":{}}
```

Consumers should select the compact file only when the current manifest lists
it, expand the nested columns before caching under the logical `cards`/`gachas`
name, and fall back to the original file from that manifest if fetching or
decoding fails. Reject uneven columns or invalid dictionary indices rather than
silently dropping rows.

Exports are generated before pruning and git publication on a JP bundle sync.
An unchanged-version poll also backfills missing exports, so deployment does not
require a new game version. Source mtimes are preserved for byte-identical bundle
members; an export at least as new as its source is reused. Generation streams
one catalog record at a time and atomically renames a temporary file. A failed
rebuild removes the stale target so it cannot be advertised alongside newer
source data; synchronization continues with the original file and later polls
retry the optional export. Failure to remove a stale target still aborts the sync.

For an offline round-trip check against local JP source files (also writes the
two exports into that fixture directory):

```sh
HARUKI_TEST_COMPACT_MASTER_DIR=/path/to/fixtures \
  cargo test --lib real_catalog_round_trip -- --ignored
```

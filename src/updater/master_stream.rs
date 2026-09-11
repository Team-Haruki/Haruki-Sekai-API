//! Streaming walk over a decrypted master-data MessagePack payload.
//!
//! A master payload (a full Nuverse master or one CP master split) is one
//! top-level map `{tableName: rows}`. Decoding it whole costs several times
//! the payload size (rmpv tree plus JSON tree plus the pretty-printed
//! output), which is what pushed the 358 MB CN master past 1.2 GB RSS on a
//! 1.6 GB account node. This walker reads the map header and then decodes
//! one entry at a time from a `Read`; array-shaped tables (the normal row
//! list) are further delivered row by row, so the peak is bounded by one row
//! of the largest table plus the output buffer. Only object-shaped tables
//! (the Nuverse `compact*` column stores) are materialized whole.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;

use serde_json::Value as JsonValue;
use tracing::warn;

use crate::client::nuverse_schema::NuverseSchemaStore;
use crate::crypto::msgpack_value_to_json;
use crate::error::AppError;

/// Receiver for the tables of one payload, in payload order.
pub trait MasterSink {
    /// An array-shaped table starts; `rows` follow, then `end_rows`.
    fn begin_rows(&mut self, key: &str, len: usize) -> Result<(), AppError>;
    fn row(&mut self, key: &str, row: JsonValue) -> Result<(), AppError>;
    fn end_rows(&mut self, key: &str) -> Result<(), AppError>;
    /// A non-array table, delivered whole.
    fn table(&mut self, key: String, value: JsonValue) -> Result<(), AppError>;
}

/// One-byte pushback so the walker can inspect a value's marker and still
/// hand the untouched bytes to the value decoder.
struct Peek<R> {
    inner: R,
    pending: Option<u8>,
}

impl<R: Read> Peek<R> {
    fn peek(&mut self) -> std::io::Result<Option<u8>> {
        if self.pending.is_none() {
            let mut b = [0u8; 1];
            if self.inner.read(&mut b)? == 1 {
                self.pending = Some(b[0]);
            }
        }
        Ok(self.pending)
    }
}

impl<R: Read> Read for Peek<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if let Some(b) = self.pending.take() {
            buf[0] = b;
            return Ok(1);
        }
        self.inner.read(buf)
    }
}

fn decode_err(what: &str, e: impl std::fmt::Display) -> AppError {
    AppError::UpstreamData(format!("{}: {}", what, e))
}

/// Walk every top-level entry of the msgpack map read from `reader` into
/// `sink`. Returns the entry count.
pub fn walk_master_payload<R: Read>(
    reader: R,
    sink: &mut impl MasterSink,
) -> Result<usize, AppError> {
    use rmp::Marker;
    let mut rd = Peek {
        inner: reader,
        pending: None,
    };
    let len = rmp::decode::read_map_len(&mut rd)
        .map_err(|e| decode_err("master payload is not a map", e))?;
    for _ in 0..len {
        let key = match rmpv::decode::read_value(&mut rd)
            .map_err(|e| decode_err("master key decode error", e))?
        {
            rmpv::Value::String(s) => s.into_str().unwrap_or_default(),
            rmpv::Value::Integer(i) => i.to_string(),
            other => {
                return Err(AppError::UpstreamData(format!(
                    "master key must be a string, got {}",
                    other
                )))
            }
        };
        let marker = rd
            .peek()?
            .map(Marker::from_u8)
            .ok_or_else(|| decode_err(&format!("master table {}", key), "unexpected EOF"))?;
        if matches!(
            marker,
            Marker::FixArray(_) | Marker::Array16 | Marker::Array32
        ) {
            let rows = rmp::decode::read_array_len(&mut rd)
                .map_err(|e| decode_err(&format!("master table {} header", key), e))?
                as usize;
            sink.begin_rows(&key, rows)?;
            for _ in 0..rows {
                let row = rmpv::decode::read_value(&mut rd).map_err(|e| {
                    decode_err(&format!("master table {} row decode error", key), e)
                })?;
                sink.row(&key, msgpack_value_to_json(&row)?)?;
            }
            sink.end_rows(&key)?;
        } else {
            let value = rmpv::decode::read_value(&mut rd)
                .map_err(|e| decode_err(&format!("master table {} decode error", key), e))?;
            let json = msgpack_value_to_json(&value)?;
            drop(value);
            sink.table(key, json)?;
        }
    }
    Ok(len as usize)
}

/// Sink that turns walked tables into `<master_dir>/<table>.json` files.
///
/// Applies the Nuverse schema restoration when a store is given (CP splits
/// pass `None`), expands `compact*` tables into their derived tables, and
/// keeps the whole-payload precedence rule: a derived table always wins over
/// a raw table of the same name regardless of payload order. Row tables are
/// pretty-printed incrementally in the exact `sonic_rs::to_string_pretty`
/// layout so the git mirror sees no formatting churn. Files are written via
/// temp+rename so readers never observe a partial table.
pub struct MasterTableWriter<'a> {
    master_dir: &'a Path,
    region_upper: String,
    schema: Option<&'a NuverseSchemaStore>,
    derived: HashSet<String>,
    open: Option<RowFile>,
    written: usize,
    skipped: usize,
}

struct RowFile {
    tmp: std::path::PathBuf,
    target: std::path::PathBuf,
    /// `None` while the table is being discarded (a derived table already won).
    writer: Option<std::io::BufWriter<std::fs::File>>,
    rows: usize,
}

impl<'a> MasterTableWriter<'a> {
    pub fn new(
        master_dir: &'a Path,
        region_upper: &str,
        schema: Option<&'a NuverseSchemaStore>,
    ) -> Self {
        Self {
            master_dir,
            region_upper: region_upper.to_string(),
            schema,
            derived: HashSet::new(),
            open: None,
            written: 0,
            skipped: 0,
        }
    }

    pub fn written(&self) -> usize {
        self.written
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }

    fn check_key(&mut self, key: &str) -> Result<(), AppError> {
        if super::master::is_safe_path_component(key) {
            return Ok(());
        }
        warn!(
            "{} Skipping master key {:?}: not a safe filename",
            self.region_upper, key
        );
        self.skipped += 1;
        Err(AppError::UpstreamData(format!(
            "master key {:?} is not a safe filename",
            key
        )))
    }

    fn write_whole(&mut self, key: &str, value: &JsonValue) -> Result<(), AppError> {
        self.check_key(key)?;
        write_master_table(self.master_dir, key, value)?;
        self.written += 1;
        Ok(())
    }
}

impl Drop for MasterTableWriter<'_> {
    fn drop(&mut self) {
        if let Some(open) = self.open.take() {
            drop(open.writer);
            let _ = std::fs::remove_file(&open.tmp);
        }
    }
}

impl MasterSink for MasterTableWriter<'_> {
    fn begin_rows(&mut self, key: &str, _len: usize) -> Result<(), AppError> {
        if self.open.is_some() {
            return Err(AppError::Internal(
                "master row table opened while another is open".to_string(),
            ));
        }
        let target = self.master_dir.join(format!("{}.json", key));
        let tmp = self
            .master_dir
            .join(format!(".{}.{}.tmp", key, uuid::Uuid::new_v4()));
        let writer = if self.derived.contains(key) {
            self.skipped += 1;
            None
        } else {
            self.check_key(key)?;
            let mut writer =
                std::io::BufWriter::with_capacity(256 * 1024, std::fs::File::create(&tmp)?);
            writer.write_all(b"[")?;
            Some(writer)
        };
        self.open = Some(RowFile {
            tmp,
            target,
            writer,
            rows: 0,
        });
        Ok(())
    }

    fn row(&mut self, key: &str, row: JsonValue) -> Result<(), AppError> {
        let Some(open) = self.open.as_mut() else {
            return Err(AppError::Internal(format!(
                "master row for {} without an open table",
                key
            )));
        };
        let Some(writer) = open.writer.as_mut() else {
            return Ok(());
        };
        let row = match self.schema {
            Some(store) => store.restore_master_row(key, row)?,
            None => row,
        };
        let pretty = sonic_rs::to_string_pretty(&row)
            .map_err(|e| AppError::ParseError(format!("serialize {} row: {}", key, e)))?;
        writer.write_all(if open.rows == 0 { b"\n  " } else { b",\n  " })?;
        for (i, line) in pretty.split('\n').enumerate() {
            if i > 0 {
                writer.write_all(b"\n  ")?;
            }
            writer.write_all(line.as_bytes())?;
        }
        open.rows += 1;
        Ok(())
    }

    fn end_rows(&mut self, key: &str) -> Result<(), AppError> {
        let Some(open) = self.open.take() else {
            return Err(AppError::Internal(format!(
                "end of master rows for {} without an open table",
                key
            )));
        };
        let Some(mut writer) = open.writer else {
            return Ok(());
        };
        let result = (|| -> Result<(), AppError> {
            if open.rows > 0 {
                writer.write_all(b"\n")?;
            }
            writer.write_all(b"]")?;
            writer.flush()?;
            drop(writer);
            std::fs::rename(&open.tmp, &open.target)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&open.tmp);
        }
        result?;
        self.written += 1;
        Ok(())
    }

    fn table(&mut self, key: String, value: JsonValue) -> Result<(), AppError> {
        let tables = match self.schema {
            Some(store) => store.restore_master_table(key, value)?,
            None => vec![(key, value)],
        };
        // `restore_master_table` yields the raw table first and any derived
        // table after it; only derived tables take precedence.
        let mut iter = tables.into_iter();
        if let Some((key, value)) = iter.next() {
            if self.derived.contains(&key) {
                self.skipped += 1;
            } else {
                self.write_whole(&key, &value)?;
            }
        }
        for (key, value) in iter {
            self.write_whole(&key, &value)?;
            self.derived.insert(key);
        }
        Ok(())
    }
}

/// Pretty-print one whole table to `<master_dir>/<key>.json` through a
/// buffered writer (no intermediate String) and rename it into place.
pub fn write_master_table(master_dir: &Path, key: &str, value: &JsonValue) -> Result<(), AppError> {
    let target = master_dir.join(format!("{}.json", key));
    let tmp = master_dir.join(format!(".{}.{}.tmp", key, uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), AppError> {
        // sonic_rs's BufferedWriter forwards every token to the inner writer,
        // so the file itself must be buffered or a 50 MB table costs millions
        // of write syscalls.
        let file = std::io::BufWriter::with_capacity(256 * 1024, std::fs::File::create(&tmp)?);
        let mut writer = sonic_rs::writer::BufferedWriter::new(file);
        sonic_rs::to_writer_pretty(&mut writer, value)
            .map_err(|e| AppError::ParseError(format!("serialize {}: {}", key, e)))?;
        writer.flush()?;
        drop(writer);
        std::fs::rename(&tmp, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::read_fill;
    use serde_json::json;

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("haruki_stream_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// Collects everything the walker delivers, re-materializing row tables.
    #[derive(Default)]
    struct Collect {
        events: Vec<String>,
        tables: Vec<(String, JsonValue)>,
        rows: Vec<JsonValue>,
    }

    impl MasterSink for Collect {
        fn begin_rows(&mut self, key: &str, len: usize) -> Result<(), AppError> {
            self.events.push(format!("begin {key} {len}"));
            self.rows.clear();
            Ok(())
        }
        fn row(&mut self, _key: &str, row: JsonValue) -> Result<(), AppError> {
            self.rows.push(row);
            Ok(())
        }
        fn end_rows(&mut self, key: &str) -> Result<(), AppError> {
            self.events.push(format!("end {key}"));
            self.tables.push((
                key.to_string(),
                JsonValue::Array(std::mem::take(&mut self.rows)),
            ));
            Ok(())
        }
        fn table(&mut self, key: String, value: JsonValue) -> Result<(), AppError> {
            self.events.push(format!("table {key}"));
            self.tables.push((key, value));
            Ok(())
        }
    }

    fn read_json(path: &Path) -> JsonValue {
        sonic_rs::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn walks_tables_in_order_streaming_rows_and_rejects_bad_payloads() {
        let master = json!({
            "cards": [{"id": 1}, {"id": 2}],
            "musics": [],
            "compactX": {"id": [1, 2]},
            "3": {"k": "v"}
        });
        let msgpack = rmp_serde::to_vec(&master).unwrap();
        let mut sink = Collect::default();
        let count = walk_master_payload(std::io::Cursor::new(msgpack), &mut sink).unwrap();
        assert_eq!(count, 4);
        assert_eq!(
            sink.events,
            vec![
                "begin cards 2",
                "end cards",
                "begin musics 0",
                "end musics",
                "table compactX",
                "table 3"
            ]
        );
        assert_eq!(
            sink.tables[0],
            ("cards".to_string(), master["cards"].clone())
        );
        assert_eq!(sink.tables[1], ("musics".to_string(), json!([])));
        assert_eq!(
            sink.tables[2],
            ("compactX".to_string(), master["compactX"].clone())
        );
        assert_eq!(sink.tables[3].0, "3");

        // Array16 / Array32 headers take the row path too.
        let mut big = Vec::new();
        rmp::encode::write_map_len(&mut big, 1).unwrap();
        rmp::encode::write_str(&mut big, "wide").unwrap();
        rmp::encode::write_array_len(&mut big, 70_000).unwrap();
        for i in 0..70_000u32 {
            rmp::encode::write_uint(&mut big, i as u64).unwrap();
        }
        let mut sink = Collect::default();
        walk_master_payload(std::io::Cursor::new(big), &mut sink).unwrap();
        assert_eq!(sink.events, vec!["begin wide 70000", "end wide"]);
        assert_eq!(sink.tables[0].1.as_array().unwrap().len(), 70_000);

        let array = rmp_serde::to_vec(&json!([1, 2])).unwrap();
        let mut sink = Collect::default();
        assert!(walk_master_payload(std::io::Cursor::new(array), &mut sink).is_err());
        assert!(walk_master_payload(std::io::Cursor::new(Vec::new()), &mut sink).is_err());
        let bad_key = {
            let mut b = Vec::new();
            rmp::encode::write_map_len(&mut b, 1).unwrap();
            rmp::encode::write_bool(&mut b, true).unwrap();
            rmp::encode::write_nil(&mut b).unwrap();
            b
        };
        assert!(walk_master_payload(std::io::Cursor::new(bad_key), &mut sink).is_err());
        let no_value = {
            let mut b = Vec::new();
            rmp::encode::write_map_len(&mut b, 1).unwrap();
            rmp::encode::write_str(&mut b, "k").unwrap();
            b
        };
        assert!(walk_master_payload(std::io::Cursor::new(no_value), &mut sink).is_err());

        // A truncated payload fails on the table that is cut off, after
        // earlier tables were delivered.
        let full = rmp_serde::to_vec(&json!({"a": [1, 2, 3], "b": [4, 5, 6]})).unwrap();
        let mut sink = Collect::default();
        let err = walk_master_payload(std::io::Cursor::new(&full[..full.len() - 2]), &mut sink)
            .unwrap_err();
        assert_eq!(sink.tables.len(), 1);
        assert!(err.to_string().contains("b"));

        // Sink errors propagate.
        struct Fail;
        impl MasterSink for Fail {
            fn begin_rows(&mut self, _: &str, _: usize) -> Result<(), AppError> {
                Err(AppError::Internal("stop".to_string()))
            }
            fn row(&mut self, _: &str, _: JsonValue) -> Result<(), AppError> {
                Ok(())
            }
            fn end_rows(&mut self, _: &str) -> Result<(), AppError> {
                Ok(())
            }
            fn table(&mut self, _: String, _: JsonValue) -> Result<(), AppError> {
                Ok(())
            }
        }
        assert!(matches!(
            walk_master_payload(std::io::Cursor::new(&full), &mut Fail).unwrap_err(),
            AppError::Internal(_)
        ));
    }

    #[test]
    fn row_streaming_output_is_byte_identical_to_whole_table_pretty_print() {
        let root = temp_dir();
        let samples = [
            json!([]),
            json!([{}]),
            json!([[], {}, 1, "s", null, true, 1.5]),
            json!([
                {"id": 1, "name": "ä 日本 \\n \"q\"", "tags": [], "obj": {}, "nested": {"a": [1, {"b": [2, 3]}]}},
                {"id": 2, "f": 0.1, "neg": -7, "big": 18446744073709551615u64, "arr": [[[]]]}
            ]),
        ];
        for (i, sample) in samples.iter().enumerate() {
            let key = format!("t{i}");
            let payload = rmp_serde::to_vec(&json!({ &key: sample })).unwrap();
            let mut writer = MasterTableWriter::new(&root, "T", None);
            walk_master_payload(std::io::Cursor::new(payload), &mut writer).unwrap();
            let streamed = std::fs::read_to_string(root.join(format!("{key}.json"))).unwrap();
            assert_eq!(
                streamed,
                sonic_rs::to_string_pretty(sample).unwrap(),
                "sample {i}"
            );
            assert_eq!(writer.written(), 1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writer_persists_tables_and_lets_derived_tables_win() {
        let root = temp_dir();
        let store =
            NuverseSchemaStore::from_slice(br#"{"schemas": [], "master": {}, "api": []}"#).unwrap();
        // Raw table arrives BEFORE its compact source: the expansion overwrites
        // it. A raw table arriving AFTER the expansion is swallowed.
        let payload = rmp_serde::to_vec(&json!({
            "widgetItems": [{"id": 999, "name": "stale"}],
            "compactWidgetItems": {"id": [1, 2], "name": ["one", "two"]},
            "widgetItems2": [{"id": 1}],
        }))
        .unwrap();
        let late = rmp_serde::to_vec(&json!({
            "widgetItems": [{"id": 42, "name": "late"}],
            "widgetItemsObj": {"k": 1},
        }))
        .unwrap();
        let mut writer = MasterTableWriter::new(&root, "CN", Some(&store));
        walk_master_payload(std::io::Cursor::new(payload), &mut writer).unwrap();
        walk_master_payload(std::io::Cursor::new(late), &mut writer).unwrap();
        assert_eq!(
            read_json(&root.join("widgetItems.json")),
            json!([{"id": 1, "name": "one"}, {"id": 2, "name": "two"}])
        );
        assert!(root.join("compactWidgetItems.json").exists());
        assert!(root.join("widgetItems2.json").exists());
        assert!(root.join("widgetItemsObj.json").exists());
        assert_eq!(writer.written(), 5);
        assert_eq!(writer.skipped(), 1);
        drop(writer);
        assert!(std::fs::read_dir(&root).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));

        // Unsafe keys are refused on both paths and leave no file behind.
        let mut plain = MasterTableWriter::new(&root, "JP", None);
        let unsafe_rows = rmp_serde::to_vec(&json!({"../escape": []})).unwrap();
        assert!(walk_master_payload(std::io::Cursor::new(unsafe_rows), &mut plain).is_err());
        let unsafe_obj = rmp_serde::to_vec(&json!({"../escape": {}})).unwrap();
        assert!(walk_master_payload(std::io::Cursor::new(unsafe_obj), &mut plain).is_err());
        assert!(!root.join("../escape.json").exists());
        assert_eq!(plain.skipped(), 2);

        // Sink misuse is reported rather than ignored, and an abandoned open
        // table leaves no temp file behind.
        let mut misuse = MasterTableWriter::new(&root, "JP", None);
        assert!(misuse.row("x", json!(1)).is_err());
        assert!(misuse.end_rows("x").is_err());
        misuse.begin_rows("x", 0).unwrap();
        assert!(misuse.begin_rows("y", 0).is_err());
        drop(misuse);
        assert!(std::fs::read_dir(&root).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));

        // Whole-table output matches the historical pretty format byte for
        // byte so git diffs stay clean across the writer change.
        let value = json!({"a": [1, {"b": null}], "c": "d"});
        write_master_table(&root, "fmt", &value).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("fmt.json")).unwrap(),
            sonic_rs::to_string_pretty(&value).unwrap()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Peak-memory probe for a Nuverse-sized payload. Generates a ~500 MB
    /// msgpack master (400 tables, one of them sized like the largest real
    /// table) straight to disk, encrypts it block by block, then decodes it
    /// with either the streaming path (default) or the legacy whole-payload
    /// path (`HARUKI_BENCH_LEGACY=1`). Run each variant under
    /// `/usr/bin/time -l` (macOS) or `/usr/bin/time -v` (Linux) and compare
    /// the maximum resident set size:
    ///
    /// ```text
    /// /usr/bin/time -l cargo test --release --lib -- --ignored master_stream::tests::peak_memory_probe
    /// HARUKI_BENCH_LEGACY=1 /usr/bin/time -l cargo test --release --lib -- --ignored master_stream::tests::peak_memory_probe
    /// ```
    #[test]
    #[ignore]
    fn peak_memory_probe() {
        use cipher::{BlockModeEncrypt, KeyIvInit};
        use std::io::{BufReader, BufWriter};

        let tables: usize = std::env::var("HARUKI_BENCH_TABLES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);
        let rows_per_table: usize = std::env::var("HARUKI_BENCH_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6_000);
        // One table is sized like the largest real one (JP `costume3ds`, ~53 MB
        // pretty JSON) so the streaming peak is measured at its true bound.
        let big_table_rows: usize = std::env::var("HARUKI_BENCH_BIG_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200_000);
        let root = temp_dir();
        let plain_path = root.join("master.msgpack");
        {
            let mut w = BufWriter::new(std::fs::File::create(&plain_path).unwrap());
            rmp::encode::write_map_len(&mut w, tables as u32).unwrap();
            for t in 0..tables {
                rmp::encode::write_str(&mut w, &format!("table{t}")).unwrap();
                let rows = if t == tables / 2 {
                    big_table_rows
                } else {
                    rows_per_table
                };
                rmp::encode::write_array_len(&mut w, rows as u32).unwrap();
                for r in 0..rows {
                    rmp::encode::write_map_len(&mut w, 8).unwrap();
                    rmp::encode::write_str(&mut w, "id").unwrap();
                    rmp::encode::write_uint(&mut w, r as u64).unwrap();
                    rmp::encode::write_str(&mut w, "seq").unwrap();
                    rmp::encode::write_uint(&mut w, (t * rows_per_table + r) as u64).unwrap();
                    rmp::encode::write_str(&mut w, "name").unwrap();
                    rmp::encode::write_str(&mut w, &format!("row-{t}-{r}-with-a-longer-name"))
                        .unwrap();
                    rmp::encode::write_str(&mut w, "assetbundleName").unwrap();
                    rmp::encode::write_str(&mut w, &format!("res{t:03}_{r:05}_rip")).unwrap();
                    rmp::encode::write_str(&mut w, "flag").unwrap();
                    rmp::encode::write_bool(&mut w, r % 2 == 0).unwrap();
                    rmp::encode::write_str(&mut w, "ratio").unwrap();
                    rmp::encode::write_f64(&mut w, r as f64 / 7.0).unwrap();
                    rmp::encode::write_str(&mut w, "tags").unwrap();
                    rmp::encode::write_array_len(&mut w, 3).unwrap();
                    for k in 0..3u64 {
                        rmp::encode::write_uint(&mut w, k).unwrap();
                    }
                    rmp::encode::write_str(&mut w, "description").unwrap();
                    rmp::encode::write_str(
                        &mut w,
                        "lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod",
                    )
                    .unwrap();
                }
            }
            w.flush().unwrap();
        }
        let plain_len = std::fs::metadata(&plain_path).unwrap().len();

        // Encrypt block by block (PKCS7 at the tail) so generation never
        // holds the payload either.
        let key = hex::decode("00112233445566778899aabbccddeeff").unwrap();
        let iv = hex::decode("ffeeddccbbaa99887766554433221100").unwrap();
        let cipher_path = root.join("master.bin");
        {
            type Enc = cbc::Encryptor<aes::Aes128>;
            let mut enc = Enc::new_from_slices(&key, &iv).unwrap();
            let mut r = BufReader::new(std::fs::File::open(&plain_path).unwrap());
            let mut w = BufWriter::new(std::fs::File::create(&cipher_path).unwrap());
            let mut buf = vec![0u8; 64 * 1024];
            let mut tail: Vec<u8> = Vec::new();
            loop {
                let n = read_fill(&mut r, &mut buf).unwrap();
                if n == 0 {
                    break;
                }
                let full = n - (n % 16);
                for block in buf[..full].chunks_exact_mut(16) {
                    enc.encrypt_block(<&mut cipher::Block<Enc>>::try_from(block).unwrap());
                }
                w.write_all(&buf[..full]).unwrap();
                tail = buf[full..n].to_vec();
                if n < buf.len() {
                    break;
                }
            }
            let pad = 16 - (tail.len() % 16);
            tail.extend(std::iter::repeat_n(pad as u8, pad));
            for block in tail.chunks_exact_mut(16) {
                enc.encrypt_block(<&mut cipher::Block<Enc>>::try_from(block).unwrap());
            }
            w.write_all(&tail).unwrap();
            w.flush().unwrap();
        }
        std::fs::remove_file(&plain_path).unwrap();

        let cryptor = crate::crypto::SekaiCryptor::from_hex(
            "00112233445566778899aabbccddeeff",
            "ffeeddccbbaa99887766554433221100",
        )
        .unwrap();
        let out = root.join("master");
        std::fs::create_dir_all(&out).unwrap();
        let started = std::time::Instant::now();
        let legacy = std::env::var("HARUKI_BENCH_LEGACY").is_ok();
        if legacy {
            let body = std::fs::read(&cipher_path).unwrap();
            let map = cryptor.unpack_ordered(&body).unwrap();
            for (k, v) in &map {
                write_master_table(&out, k, v).unwrap();
            }
        } else {
            let reader = cryptor.decrypt_reader(BufReader::with_capacity(
                1 << 20,
                std::fs::File::open(&cipher_path).unwrap(),
            ));
            let mut writer = MasterTableWriter::new(&out, "BENCH", None);
            let count = walk_master_payload(reader, &mut writer).unwrap();
            assert_eq!(count, tables);
        }
        let files = std::fs::read_dir(&out).unwrap().count();
        assert_eq!(files, tables);
        eprintln!(
            "peak_memory_probe: mode={} payload={} MB tables={} elapsed={:?}",
            if legacy { "legacy" } else { "streaming" },
            plain_len / (1024 * 1024),
            tables,
            started.elapsed()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

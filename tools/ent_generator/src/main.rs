use regex::Regex;
use std::fs;
use std::path::Path;

/// Reads a Rust model file and extracts the root struct name from `pub type XXX = Vec<YYY>;`.
/// Returns (table_name_lowercase, root_struct_name) or None if no root type alias is found.
fn extract_root_type(file_content: &str) -> Option<(String, String)> {
    let re = Regex::new(r"pub type (\w+) = Vec<(\w+)>;").unwrap();
    if let Some(caps) = re.captures(file_content) {
        let type_alias = caps.get(1).unwrap().as_str(); // e.g. "Shopitem"
        let root_struct = caps.get(2).unwrap().as_str(); // e.g. "ShopitemElement"
        let table_name = type_alias.to_lowercase(); // e.g. "shopitem"
        Some((table_name, root_struct.to_string()))
    } else {
        None
    }
}

/// Extracts fields from a specific struct definition, returning (field_name, field_type) pairs.
fn extract_struct_fields(file_content: &str, struct_name: &str) -> Vec<(String, String)> {
    // Find the struct block
    let struct_pattern = format!(r"pub struct {} \{{", regex::escape(struct_name));
    let struct_re = Regex::new(&struct_pattern).unwrap();

    let start = match struct_re.find(file_content) {
        Some(m) => m.end(),
        None => return Vec::new(),
    };

    // Find the matching closing brace
    let mut depth = 1;
    let mut end = start;
    for (i, ch) in file_content[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i;
                    break;
                }
            }
            _ => {}
        }
    }

    let struct_body = &file_content[start..end];

    // Parse `pub field_name: Type` lines
    let field_re = Regex::new(r"pub (\w+)\s*:\s*(.+?)\s*,?\s*$").unwrap();
    let mut fields = Vec::new();

    for line in struct_body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub ") {
            if let Some(caps) = field_re.captures(trimmed) {
                let name = caps.get(1).unwrap().as_str().to_string();
                let typ = caps.get(2).unwrap().as_str().trim().to_string();
                fields.push((name, typ));
            }
        }
    }

    fields
}

/// Maps a Rust type to an EntGo-compatible type string.
fn rust_type_to_ent_type(rust_type: &str) -> String {
    let inner = if rust_type.starts_with("Option<") && rust_type.ends_with('>') {
        &rust_type[7..rust_type.len() - 1]
    } else {
        rust_type
    };

    match inner {
        "i64" | "i32" | "u64" | "u32" => "int64".to_string(),
        "f64" | "f32" => "float64".to_string(),
        "bool" => "bool".to_string(),
        "String" => "string".to_string(),
        _ => {
            // Anything complex (Vec, nested struct, serde_json::Value, etc.) → JSON
            "json.RawMessage".to_string()
        }
    }
}

/// Converts a camelCase field name to snake_case.
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

fn main() {
    let models_dir = "../../src/models";
    let output_dir = "../../schema_info_generated.json";

    let models_path = Path::new(models_dir);
    if !models_path.exists() {
        eprintln!("Models directory not found: {}", models_dir);
        std::process::exit(1);
    }

    let mut tables: Vec<serde_json::Value> = Vec::new();
    let mut processed = 0;
    let mut skipped = 0;

    let mut entries: Vec<_> = fs::read_dir(models_path)
        .expect("Failed to read models directory")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        if path.file_stem().unwrap().to_str().unwrap() == "mod" {
            continue;
        }

        let content = fs::read_to_string(&path).unwrap_or_default();

        let (table_name, root_struct) = match extract_root_type(&content) {
            Some(v) => v,
            None => {
                skipped += 1;
                eprintln!(
                    "SKIP: {} (no pub type alias found)",
                    path.file_name().unwrap().to_str().unwrap()
                );
                continue;
            }
        };

        let fields = extract_struct_fields(&content, &root_struct);
        if fields.is_empty() {
            skipped += 1;
            eprintln!(
                "SKIP: {} (struct {} has no fields)",
                path.file_name().unwrap().to_str().unwrap(),
                root_struct
            );
            continue;
        }

        // Build column definitions
        let mut columns: Vec<String> = Vec::new();
        let mut has_id = false;

        for (field_name, field_type) in &fields {
            let db_col = to_snake_case(field_name);
            let ent_type = rust_type_to_ent_type(field_type);

            if db_col == "id" {
                has_id = true;
                // Rename "id" to "game_id" to avoid collision with auto-increment primary key
                columns.push(format!("game_id:{}", ent_type));
            } else {
                columns.push(format!("{}:{}", db_col, ent_type));
            }
        }

        // Always add server_region column
        columns.push("server_region:string".to_string());

        // Determine unique keys: if has game_id, use [game_id, server_region], else [server_region]
        let unique_keys = if has_id {
            serde_json::json!([["id", "server_region"]])
        } else {
            serde_json::json!([["server_region"]])
        };

        // Pluralize table name for consistency
        let table_name_plural = if table_name.ends_with('s') {
            table_name.clone()
        } else if table_name.ends_with("ch")
            || table_name.ends_with("sh")
            || table_name.ends_with("ss")
            || table_name.ends_with("x")
            || table_name.ends_with("z")
        {
            format!("{}es", table_name)
        } else if table_name.ends_with('y')
            && !table_name.ends_with("ay")
            && !table_name.ends_with("ey")
            && !table_name.ends_with("oy")
            && !table_name.ends_with("uy")
        {
            format!("{}ies", &table_name[..table_name.len() - 1])
        } else {
            format!("{}s", table_name)
        };

        tables.push(serde_json::json!({
            "name": table_name_plural,
            "columns": columns,
            "unique_keys": unique_keys,
        }));

        processed += 1;
        println!(
            "OK: {} -> table '{}' ({} columns)",
            path.file_name().unwrap().to_str().unwrap(),
            table_name_plural,
            columns.len()
        );
    }

    // Write output
    let json = serde_json::to_string_pretty(&tables).unwrap();
    fs::write(output_dir, &json).expect("Failed to write output file");

    println!("\n=== Summary ===");
    println!("Processed: {}", processed);
    println!("Skipped:   {}", skipped);
    println!("Output:    {}", output_dir);
}

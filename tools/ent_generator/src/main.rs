use regex::Regex;
use std::fs;
use std::path::Path;

/// Returns overridden unique keys for tables where the default (game_id + server_region)
/// is incorrect. Determined by analyzing actual JSON data across all 5 regions.
/// All keys use DB column names (e.g. "game_id" not "id").
/// Returns None to use default logic, Some(vec) to override.
fn unique_key_override(table_name: &str) -> Option<serde_json::Value> {
    let keys: Option<Vec<Vec<&str>>> = match table_name {
        // Tables without id field — natural composite keys
        "areaitemlevel" => Some(vec![vec!["area_item_id", "level", "server_region"]]),
        "cardcostume3d" => Some(vec![vec!["costume3_d_id", "server_region"]]),
        "cardraritie" => Some(vec![vec!["card_rarity_type", "server_region"]]),
        "eventmusic" => Some(vec![vec!["event_id", "music_id", "server_region"]]),
        "masterlesson" => Some(vec![vec![
            "card_rarity_type",
            "master_rank",
            "server_region",
        ]]),
        "worldbloomdifferentattributebonuse" => {
            Some(vec![vec!["attribute_count", "server_region"]])
        }
        "worldbloomsupportdeckbonuse" => Some(vec![vec!["card_rarity_type", "server_region"]]),
        // Tables where id exists in model but is missing/non-unique in JSON data
        "eventcard" => Some(vec![vec!["card_id", "event_id", "server_region"]]),
        "level" => Some(vec![vec!["level_type", "level", "server_region"]]),
        "musictag" => Some(vec![vec!["music_id", "music_tag", "server_region"]]),
        "charactermissionv2parametergroup" => Some(vec![vec!["game_id", "seq", "server_region"]]),
        "resourceboxe" => Some(vec![vec![
            "resource_box_purpose",
            "game_id",
            "server_region",
        ]]),
        // ngwords: data has genuine duplicates, no unique key possible
        "ngword" => Some(vec![]),
        // resourceBoxDetails (Nuverse regions only): rows carry neither id nor seq and
        // duplicate on every column but resource_quantity, so no unique key is declared
        "resourceboxdetail" => Some(vec![]),
        _ => None,
    };
    keys.map(|k| serde_json::json!(k))
}

/// Non-unique indexes Haruki-Cloud adds by hand on top of the generated schema, keyed
/// like `unique_key_override`. Emitting them here keeps a regenerated Cloud schema
/// identical to the hand-edited one. Column names are DB names (`id` is the ent PK,
/// not the game id); they only reach the Go output, never `schema_info.json`.
fn secondary_index_override(table_name: &str) -> Vec<Vec<&'static str>> {
    match table_name {
        // Cloud loads a region's difficulties per music.
        "musicdifficultie" => vec![vec!["server_region", "music_id"]],
        // No game_id; Cloud reads a whole region ordered by the ent PK, which is the
        // insert order and therefore the display order of a box's contents.
        "resourceboxdetail" => vec![vec!["server_region", "id"]],
        _ => Vec::new(),
    }
}

/// Reads a Rust model file and extracts the root struct name from `pub type XXX = Vec<YYY>;`.
/// Returns (table_name_lowercase, root_struct_name) or None if no root type alias is found.
fn extract_root_type(file_content: &str) -> Option<(String, String)> {
    let re = Regex::new(r"pub\s+type\s+(\w+)\s*=\s*Vec\s*<\s*(\w+)\s*>\s*;").unwrap();
    if let Some(caps) = re.captures(file_content) {
        let type_alias = caps.get(1).unwrap().as_str(); // e.g. "Shopitem"
        let root_struct = caps.get(2).unwrap().as_str(); // e.g. "ShopitemElement"
        let table_name = type_alias.to_lowercase(); // e.g. "shopitem"
        Some((table_name, root_struct.to_string()))
    } else {
        None
    }
}

/// Derives the EntGo schema name (PascalCase singular) from the root struct name.
fn derive_schema_name(root_struct: &str) -> String {
    if let Some(schema_name) = root_struct.strip_suffix("Element") {
        schema_name.to_string()
    } else {
        root_struct.to_string()
    }
}

/// Generates an EntGo schema file with an explicit table name annotation.
fn generate_ent_go_schema(
    schema_name: &str,
    table_name: &str,
    columns: &[String],
    unique_keys_json: &serde_json::Value,
    secondary_indexes: &[Vec<&str>],
) -> String {
    let mut fields_code = Vec::new();
    let mut needs_json_import = false;
    for col_def in columns {
        let Some((field, uses_json)) = generate_field_line(col_def) else {
            continue;
        };
        fields_code.push(field);
        needs_json_import |= uses_json;
    }
    let mut index_lines = generate_index_lines(unique_keys_json);
    index_lines.extend(secondary_indexes.iter().map(|keys| {
        let quoted = keys
            .iter()
            .map(|key| format!("\"{}\"", key))
            .collect::<Vec<_>>();
        format!("\t\tindex.Fields({}),", quoted.join(", "))
    }));
    let has_indexes = !index_lines.is_empty();
    let imports = generate_imports(needs_json_import, has_indexes);

    let mut out = String::new();
    out.push_str("// Code generated by ent_generator. DO NOT EDIT.\n");
    out.push_str("package schema\n\n");
    out.push_str("import (\n");
    out.push_str(&imports.join("\n"));
    out.push_str("\n)\n\n");

    // Schema struct
    out.push_str(&format!(
        "type {} struct {{\n\tent.Schema\n}}\n\n",
        schema_name
    ));

    // Fields method
    out.push_str(&format!(
        "func ({}) Fields() []ent.Field {{\n\treturn []ent.Field{{\n",
        schema_name
    ));
    out.push_str(&fields_code.join("\n"));
    out.push_str("\n\t}\n}\n\n");

    // Annotations method with explicit table name
    out.push_str(&format!(
        "func ({}) Annotations() []schema.Annotation {{\n\treturn []schema.Annotation{{\n\t\tentsql.Annotation{{Table: \"{}\"}},\n\t}}\n}}\n",
        schema_name, table_name
    ));

    // Indexes method
    if has_indexes {
        out.push_str(&format!(
            "\nfunc ({}) Indexes() []ent.Index {{\n\treturn []ent.Index{{\n",
            schema_name
        ));
        out.push_str(&index_lines.join("\n"));
        out.push_str("\n\t}\n}\n");
    }

    out
}

fn generate_field_line(column: &str) -> Option<(String, bool)> {
    let (name, column_type) = column.split_once(':')?;
    let optional = if name == "server_region" {
        ""
    } else {
        ".Optional()"
    };
    let line = match column_type {
        "int64" => format!("\t\tfield.Int64(\"{}\"){},", name, optional),
        "int32" | "int" => format!("\t\tfield.Int(\"{}\"){},", name, optional),
        "float64" | "float32" | "float" => {
            format!("\t\tfield.Float(\"{}\"){},", name, optional)
        }
        "bool" => format!("\t\tfield.Bool(\"{}\"){},", name, optional),
        "json.RawMessage" => format!(
            "\t\tfield.JSON(\"{}\", json.RawMessage{{}}){},",
            name, optional
        ),
        _ => format!("\t\tfield.String(\"{}\"){},", name, optional),
    };
    Some((line, column_type == "json.RawMessage"))
}

fn generate_index_lines(unique_keys: &serde_json::Value) -> Vec<String> {
    unique_keys
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_array)
        .filter_map(|keys| {
            let mapped = keys
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|key| if key == "id" { "game_id" } else { key })
                .map(|key| format!("\"{}\"", key))
                .collect::<Vec<_>>();
            (!mapped.is_empty())
                .then(|| format!("\t\tindex.Fields({}).Unique(),", mapped.join(", ")))
        })
        .collect()
}

fn generate_imports(needs_json: bool, has_indexes: bool) -> Vec<String> {
    let mut imports = Vec::new();
    if needs_json {
        imports.extend(["\t\"encoding/json\"".to_string(), String::new()]);
    }
    imports.extend([
        "\t\"entgo.io/ent\"".to_string(),
        "\t\"entgo.io/ent/dialect/entsql\"".to_string(),
        "\t\"entgo.io/ent/schema\"".to_string(),
        "\t\"entgo.io/ent/schema/field\"".to_string(),
    ]);
    if has_indexes {
        imports.push("\t\"entgo.io/ent/schema/index\"".to_string());
    }
    imports
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

    // Parse `pub field_name: Type,` declarations. rustfmt wraps a long one after the
    // colon (`pub name:\n    Option<Vec<T>>,`), so continuation lines are joined
    // until the declaration ends with a comma.
    let field_re = Regex::new(r"pub (\w+)\s*:\s*(.+?)\s*,?\s*$").unwrap();
    let mut fields = Vec::new();
    let mut pending: Option<String> = None;

    for line in struct_body.lines() {
        let trimmed = line.trim();
        let declaration = match pending.take() {
            Some(mut open) => {
                open.push(' ');
                open.push_str(trimmed);
                open
            }
            None if trimmed.starts_with("pub ") => trimmed.to_string(),
            None => continue,
        };
        if !declaration.ends_with(',') {
            pending = Some(declaration);
            continue;
        }
        push_field(&field_re, &declaration, &mut fields);
    }
    if let Some(declaration) = pending {
        push_field(&field_re, &declaration, &mut fields);
    }

    fields
}

fn push_field(field_re: &Regex, declaration: &str, fields: &mut Vec<(String, String)>) {
    if let Some(caps) = field_re.captures(declaration) {
        let name = caps.get(1).unwrap().as_str().to_string();
        let typ = caps.get(2).unwrap().as_str().trim().to_string();
        if !typ.is_empty() {
            fields.push((name, typ));
        }
    }
}

/// Extracts names of simple enums (all unit variants, no data) from a Rust source file.
/// These enums serialize as plain strings via serde.
fn extract_simple_enums(file_content: &str) -> std::collections::HashSet<String> {
    let mut result = std::collections::HashSet::new();
    let enum_re = Regex::new(r"pub enum (\w+)\s*\{([^}]+)\}").unwrap();
    for caps in enum_re.captures_iter(file_content) {
        let name = caps.get(1).unwrap().as_str();
        let body = caps.get(2).unwrap().as_str();
        // A simple enum has NO tuple variants `Variant(...)` or struct variants `Variant {...}`
        let has_data_variant = body.lines().any(|line| {
            let trimmed = line.trim();
            // Skip attributes, comments, and empty lines
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.starts_with("//")
                || trimmed.starts_with(']')
            {
                return false;
            }
            trimmed.contains('(') || trimmed.contains('{')
        });
        if !has_data_variant {
            result.insert(name.to_string());
        }
    }
    result
}

/// Maps a Rust type to an EntGo-compatible type string.
/// `simple_enums` is the set of enum names that serialize as plain strings.
fn rust_type_to_ent_type(
    rust_type: &str,
    simple_enums: &std::collections::HashSet<String>,
) -> String {
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
            if simple_enums.contains(inner) {
                "string".to_string()
            } else {
                "json.RawMessage".to_string()
            }
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
    let schema_output = "../../schema_info_generated.json";
    let ent_output_dir = "../../ent_schemas/generated";

    let models_path = Path::new(models_dir);
    if !models_path.exists() {
        eprintln!("Models directory not found: {}", models_dir);
        std::process::exit(1);
    }

    let ent_path = Path::new(ent_output_dir);
    fs::create_dir_all(ent_path).expect("Failed to create EntGo output directory");

    let mut entries: Vec<_> = fs::read_dir(models_path)
        .expect("Failed to read models directory")
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let mut tables = Vec::new();
    let mut skipped = 0;
    for entry in entries {
        let path = entry.path();
        if !is_model_file(&path) {
            continue;
        }
        match process_model(&path, ent_path) {
            Some(table) => tables.push(table),
            None => skipped += 1,
        }
    }
    let json = serde_json::to_string_pretty(&tables).unwrap();
    fs::write(schema_output, &json).expect("Failed to write output file");
    let processed = tables.len();
    println!("\n=== Summary ===");
    println!("Processed: {}", processed);
    println!("Skipped:   {}", skipped);
    println!("Schema JSON: {}", schema_output);
    println!("Go schemas:  {} files in {}", processed, ent_output_dir);
}

fn is_model_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("rs")
        && path.file_stem().and_then(|stem| stem.to_str()) != Some("mod")
}

fn process_model(path: &Path, ent_path: &Path) -> Option<serde_json::Value> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let (table_name, root_struct) = extract_root_type(&content).or_else(|| {
        eprintln!("SKIP: {} (no pub type alias found)", display_name(path));
        None
    })?;
    let fields = extract_struct_fields(&content, &root_struct);
    if fields.is_empty() {
        eprintln!(
            "SKIP: {} (struct {} has no fields)",
            display_name(path),
            root_struct
        );
        return None;
    }
    let simple_enums = extract_simple_enums(&content);
    let (mut columns, has_id) = build_columns(&fields, &simple_enums);
    columns.push("server_region:string".to_string());
    let unique_keys = unique_key_override(&table_name).unwrap_or_else(|| {
        if has_id {
            serde_json::json!([["id", "server_region"]])
        } else {
            serde_json::json!([])
        }
    });
    let secondary_indexes = secondary_index_override(&table_name);
    let table_name = pluralize_table_name(&table_name);
    let schema_name = derive_schema_name(&root_struct);
    let go_code = generate_ent_go_schema(
        &schema_name,
        &table_name,
        &columns,
        &unique_keys,
        &secondary_indexes,
    );
    fs::write(ent_path.join(format!("{}.go", table_name)), go_code)
        .expect("Failed to write Go schema file");
    println!(
        "OK: {} -> table '{}' / schema '{}' ({} columns)",
        display_name(path),
        table_name,
        schema_name,
        columns.len()
    );
    Some(serde_json::json!({
        "name": table_name,
        "columns": columns,
        "unique_keys": unique_keys,
    }))
}

fn display_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unknown>")
}

fn build_columns(
    fields: &[(String, String)],
    simple_enums: &std::collections::HashSet<String>,
) -> (Vec<String>, bool) {
    let mut has_id = false;
    let columns = fields
        .iter()
        .map(|(field_name, field_type)| {
            let column = to_snake_case(field_name);
            let column = if column == "id" {
                has_id = true;
                "game_id".to_string()
            } else {
                column
            };
            format!(
                "{}:{}",
                column,
                rust_type_to_ent_type(field_type, simple_enums)
            )
        })
        .collect();
    (columns, has_id)
}

fn pluralize_table_name(name: &str) -> String {
    if name.ends_with('s') {
        name.to_string()
    } else if ["ch", "sh", "ss", "x", "z"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        format!("{}es", name)
    } else if name.ends_with('y')
        && !["ay", "ey", "oy", "uy"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
    {
        format!("{}ies", &name[..name.len() - 1])
    } else {
        format!("{}s", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_fields_wrapped_by_rustfmt() {
        let source = r#"
pub struct SampleElement {
    pub card_rarity_type: Option<String>,

    pub world_bloom_support_deck_character_bonuses:
        Option<Vec<WorldBloomSupportDeckCharacterBonus>>,

    pub bonus_rate: Option<f64>,
    pub last: Option<i64>
}
"#;
        let fields = extract_struct_fields(source, "SampleElement");
        assert_eq!(
            fields,
            vec![
                ("card_rarity_type".to_string(), "Option<String>".to_string()),
                (
                    "world_bloom_support_deck_character_bonuses".to_string(),
                    "Option<Vec<WorldBloomSupportDeckCharacterBonus>>".to_string()
                ),
                ("bonus_rate".to_string(), "Option<f64>".to_string()),
                ("last".to_string(), "Option<i64>".to_string()),
            ]
        );
        let simple_enums = std::collections::HashSet::new();
        let (columns, has_id) = build_columns(&fields, &simple_enums);
        assert!(!has_id);
        assert_eq!(
            columns[1],
            "world_bloom_support_deck_character_bonuses:json.RawMessage"
        );
        assert_eq!(columns[2], "bonus_rate:float64");
    }

    #[test]
    fn pluralizes_like_the_ingest_engine_expects() {
        assert_eq!(pluralize_table_name("omikuji"), "omikujis");
        assert_eq!(pluralize_table_name("musiccategorie"), "musiccategories");
        assert_eq!(
            pluralize_table_name("streaminglivecategory"),
            "streaminglivecategories"
        );
        assert_eq!(pluralize_table_name("cards"), "cards");
        assert_eq!(pluralize_table_name("box"), "boxes");
    }

    #[test]
    fn secondary_indexes_reach_the_go_schema_but_not_unique_keys() {
        let unique = serde_json::json!([["id", "server_region"]]);
        let go = generate_ent_go_schema(
            "Musicdifficultie",
            "musicdifficulties",
            &[
                "game_id:int64".to_string(),
                "server_region:string".to_string(),
            ],
            &unique,
            &secondary_index_override("musicdifficultie"),
        );
        assert!(go.contains("index.Fields(\"game_id\", \"server_region\").Unique(),"));
        assert!(go.contains("index.Fields(\"server_region\", \"music_id\"),"));
        assert!(secondary_index_override("cards").is_empty());
        let go = generate_ent_go_schema(
            "Resourceboxdetail",
            "resourceboxdetails",
            &[
                "resource_box_id:int64".to_string(),
                "server_region:string".to_string(),
            ],
            &serde_json::json!([]),
            &secondary_index_override("resourceboxdetail"),
        );
        assert!(!go.contains(".Unique()"));
        assert!(go.contains("index.Fields(\"server_region\", \"id\"),"));
        assert!(go.contains("entgo.io/ent/schema/index"));
    }
}

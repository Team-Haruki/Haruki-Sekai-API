use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map as JsonMap, Number, Value as JsonValue};

use crate::error::AppError;

#[derive(Debug, Clone, Deserialize)]
pub struct NuverseSchemaBundle {
    #[serde(default)]
    schemas: Vec<JsonValue>,
    #[serde(default)]
    master: HashMap<String, String>,
    #[serde(default)]
    api: Vec<ApiSchemaMapping>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiSchemaMapping {
    path: String,
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    fields: Vec<ApiFieldMapping>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiFieldMapping {
    selector: String,
    schema: String,
}

#[derive(Debug, Clone)]
pub struct NuverseSchemaStore {
    registry: Arc<Registry>,
    master: HashMap<String, String>,
    api: Vec<ApiSchemaMapping>,
}

type Registry = HashMap<String, Arc<Schema>>;

#[derive(Debug, Clone)]
struct Schema {
    kind: SchemaKind,
    name: Option<String>,
    fields: Vec<Field>,
    items: Option<Arc<Schema>>,
    values: Option<Arc<Schema>>,
    union_of: Vec<Arc<Schema>>,
    union_dispatch: Vec<UnionVariant>,
}

#[derive(Debug, Clone)]
enum SchemaKind {
    Null,
    Boolean,
    Int,
    Long,
    Float,
    Double,
    Bytes,
    String,
    Record,
    Array,
    Map,
    Union,
    Ref,
    Any,
}

#[derive(Debug, Clone)]
struct Field {
    name: String,
    ty: Arc<Schema>,
    key: MsgpackKey,
}

#[derive(Debug, Clone)]
enum MsgpackKey {
    String(String),
    Int(i64),
}

#[derive(Debug, Clone)]
struct UnionVariant {
    key: i64,
    ty: String,
}

impl NuverseSchemaStore {
    pub fn from_slice(data: &[u8]) -> Result<Self, AppError> {
        let bundle: NuverseSchemaBundle = serde_json::from_slice(data)
            .map_err(|e| AppError::ParseError(format!("Nuverse schema bundle: {}", e)))?;
        Self::from_bundle(bundle)
    }

    pub fn from_bundle(bundle: NuverseSchemaBundle) -> Result<Self, AppError> {
        let mut builder = SchemaBuilder::default();
        for schema in &bundle.schemas {
            builder.parse_root(schema)?;
        }
        let registry = builder.finish();
        Ok(Self {
            registry: Arc::new(registry),
            master: bundle.master,
            api: bundle.api,
        })
    }

    pub fn restore_master_msgpack(
        &self,
        msgpack: &[u8],
    ) -> Result<IndexMap<String, JsonValue>, AppError> {
        let raw = read_msgpack_json(msgpack)?;
        let raw_obj = raw.as_object().ok_or_else(|| {
            AppError::ParseError("Nuverse master payload must be an object".to_string())
        })?;
        let mut restored = IndexMap::with_capacity(raw_obj.len());
        for (key, value) in raw_obj {
            if let Some(schema_name) = self.master.get(key) {
                if let Some(schema) = self.schema(schema_name) {
                    restored.insert(
                        key.clone(),
                        restore_master_value(schema, value, &self.registry)?,
                    );
                    continue;
                }
            }
            restored.insert(key.clone(), value.clone());
        }
        Ok(restored)
    }

    pub fn restore_api_json(&self, path: &str, value: JsonValue) -> Result<JsonValue, AppError> {
        let Some(mapping) = self.api_mapping_for_path(path) else {
            return Ok(value);
        };
        let mut restored = if let Some(schema_name) = &mapping.schema {
            if let Some(schema) = self.schema(schema_name) {
                restore_json(schema, &value, &self.registry)?
            } else {
                value
            }
        } else {
            value
        };
        for field in &mapping.fields {
            if let Some(schema) = self.schema(&field.schema) {
                restore_selector(&mut restored, &field.selector, schema, &self.registry)?;
            }
        }
        Ok(restored)
    }

    fn api_mapping_for_path(&self, path: &str) -> Option<&ApiSchemaMapping> {
        self.api
            .iter()
            .filter(|mapping| path_matches(&mapping.path, path))
            .max_by_key(|mapping| mapping.path.len())
    }

    fn schema(&self, name: &str) -> Option<&Arc<Schema>> {
        self.registry.get(name)
    }
}

fn restore_master_value(
    schema: &Arc<Schema>,
    value: &JsonValue,
    registry: &Registry,
) -> Result<JsonValue, AppError> {
    if let Some(arr) = value.as_array() {
        return arr
            .iter()
            .map(|item| restore_json(schema, item, registry))
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::Array);
    }
    restore_json(schema, value, registry)
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let path = path.split_once('?').map(|(p, _)| p).unwrap_or(path);
    let pattern = pattern.split_once('?').map(|(p, _)| p).unwrap_or(pattern);
    if pattern == path {
        return true;
    }
    let pattern_parts: Vec<_> = pattern.trim_matches('/').split('/').collect();
    let path_parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if pattern_parts.len() != path_parts.len() {
        return false;
    }
    pattern_parts
        .iter()
        .zip(path_parts.iter())
        .all(|(p, v)| (p.starts_with('{') && p.ends_with('}')) || *p == *v)
}

#[derive(Default)]
struct SchemaBuilder {
    registry: Registry,
}

impl SchemaBuilder {
    fn parse_root(&mut self, raw: &JsonValue) -> Result<Arc<Schema>, AppError> {
        if let Some(arr) = raw.as_array() {
            let mut first = None;
            for item in arr {
                let parsed = self.parse_schema(item)?;
                if first.is_none() {
                    first = Some(parsed);
                }
            }
            return first.ok_or_else(|| AppError::ParseError("empty schema array".to_string()));
        }
        self.parse_schema(raw)
    }

    fn parse_schema(&mut self, raw: &JsonValue) -> Result<Arc<Schema>, AppError> {
        match raw {
            JsonValue::String(name) => Ok(self.primitive_or_ref(name)),
            JsonValue::Array(items) => {
                let union_of = items
                    .iter()
                    .map(|item| self.parse_schema(item))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Arc::new(Schema {
                    kind: SchemaKind::Union,
                    name: None,
                    fields: Vec::new(),
                    items: None,
                    values: None,
                    union_of,
                    union_dispatch: Vec::new(),
                }))
            }
            JsonValue::Object(obj) => self.parse_object(obj),
            _ => Ok(Arc::new(any_schema())),
        }
    }

    fn parse_object(&mut self, obj: &JsonMap<String, JsonValue>) -> Result<Arc<Schema>, AppError> {
        let Some(ty) = obj.get("type") else {
            return Ok(Arc::new(null_schema()));
        };
        if ty.is_array() {
            return self.parse_schema(ty);
        }
        let ty = ty.as_str().unwrap_or("null");
        match ty {
            "record" => self.parse_record(obj),
            "array" => {
                let items = obj
                    .get("items")
                    .map(|v| self.parse_schema(v))
                    .transpose()?
                    .unwrap_or_else(|| Arc::new(any_schema()));
                Ok(Arc::new(Schema {
                    kind: SchemaKind::Array,
                    name: None,
                    fields: Vec::new(),
                    items: Some(items),
                    values: None,
                    union_of: Vec::new(),
                    union_dispatch: Vec::new(),
                }))
            }
            "map" => {
                let values = obj
                    .get("values")
                    .map(|v| self.parse_schema(v))
                    .transpose()?
                    .unwrap_or_else(|| Arc::new(any_schema()));
                Ok(Arc::new(Schema {
                    kind: SchemaKind::Map,
                    name: None,
                    fields: Vec::new(),
                    items: None,
                    values: Some(values),
                    union_of: Vec::new(),
                    union_dispatch: Vec::new(),
                }))
            }
            primitive => Ok(self.primitive_or_ref(primitive)),
        }
    }

    fn parse_record(&mut self, obj: &JsonMap<String, JsonValue>) -> Result<Arc<Schema>, AppError> {
        let short_name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let namespace = obj
            .get("namespace")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let full_name = if !namespace.is_empty() && !short_name.contains('.') {
            format!("{namespace}.{short_name}")
        } else {
            short_name.clone()
        };

        let fields = obj
            .get("fields")
            .and_then(|v| v.as_array())
            .map(|raw_fields| {
                raw_fields
                    .iter()
                    .map(|field| self.parse_field(field))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        let union_dispatch = obj
            .get("msgpack_unions")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let key = item.get("key").and_then(|v| v.as_i64())?;
                        let ty = item.get("type").and_then(|v| v.as_str())?.to_string();
                        Some(UnionVariant { key, ty })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let schema = Arc::new(Schema {
            kind: SchemaKind::Record,
            name: Some(full_name.clone()),
            fields,
            items: None,
            values: None,
            union_of: Vec::new(),
            union_dispatch,
        });
        if !full_name.is_empty() {
            self.registry.insert(full_name, schema.clone());
        }
        if !short_name.is_empty() {
            self.registry.insert(short_name, schema.clone());
        }
        Ok(schema)
    }

    fn parse_field(&mut self, raw: &JsonValue) -> Result<Field, AppError> {
        let obj = raw
            .as_object()
            .ok_or_else(|| AppError::ParseError("Avro field must be an object".to_string()))?;
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::ParseError("Avro field missing name".to_string()))?
            .to_string();
        let ty = obj
            .get("type")
            .map(|v| self.parse_schema(v))
            .transpose()?
            .unwrap_or_else(|| Arc::new(any_schema()));
        let key = match obj.get("msgpack_key") {
            Some(JsonValue::Number(n)) => MsgpackKey::Int(n.as_i64().unwrap_or_default()),
            Some(JsonValue::String(s)) => MsgpackKey::String(s.clone()),
            _ => MsgpackKey::String(name.clone()),
        };
        Ok(Field { name, ty, key })
    }

    fn primitive_or_ref(&self, name: &str) -> Arc<Schema> {
        let kind = match name {
            "null" => SchemaKind::Null,
            "boolean" => SchemaKind::Boolean,
            "int" => SchemaKind::Int,
            "long" => SchemaKind::Long,
            "float" => SchemaKind::Float,
            "double" => SchemaKind::Double,
            "bytes" => SchemaKind::Bytes,
            "string" => SchemaKind::String,
            _ => {
                if let Some(schema) = self.registry.get(name) {
                    return schema.clone();
                }
                SchemaKind::Ref
            }
        };
        let is_ref = matches!(kind, SchemaKind::Ref);
        Arc::new(Schema {
            kind,
            name: is_ref.then(|| name.to_string()),
            fields: Vec::new(),
            items: None,
            values: None,
            union_of: Vec::new(),
            union_dispatch: Vec::new(),
        })
    }

    fn finish(self) -> Registry {
        self.registry
    }
}

fn restore_json(
    schema: &Arc<Schema>,
    value: &JsonValue,
    registry: &Registry,
) -> Result<JsonValue, AppError> {
    let schema = resolve_schema(schema, registry);
    match schema.kind {
        SchemaKind::Null => Ok(JsonValue::Null),
        SchemaKind::Boolean
        | SchemaKind::Int
        | SchemaKind::Long
        | SchemaKind::Float
        | SchemaKind::Double
        | SchemaKind::Bytes
        | SchemaKind::String
        | SchemaKind::Any
        | SchemaKind::Ref => Ok(value.clone()),
        SchemaKind::Array => restore_array(&schema, value, registry),
        SchemaKind::Map => restore_map(&schema, value, registry),
        SchemaKind::Union => restore_union(&schema, value, registry),
        SchemaKind::Record => restore_record(&schema, value, registry),
    }
}

fn restore_record(
    schema: &Schema,
    value: &JsonValue,
    registry: &Registry,
) -> Result<JsonValue, AppError> {
    if !schema.union_dispatch.is_empty() {
        return restore_union_dispatch(schema, value, registry);
    }
    if let Some(arr) = value.as_array() {
        let mut out = JsonMap::new();
        let mut by_index: HashMap<i64, &Field> = HashMap::new();
        for field in &schema.fields {
            if let MsgpackKey::Int(idx) = field.key {
                by_index.insert(idx, field);
            }
        }
        for (idx, item) in arr.iter().enumerate() {
            if item.is_null() {
                continue;
            }
            if let Some(field) = by_index.get(&(idx as i64)) {
                out.insert(field.name.clone(), restore_json(&field.ty, item, registry)?);
            }
        }
        return Ok(JsonValue::Object(out));
    }
    if let Some(obj) = value.as_object() {
        let mut out = obj.clone();
        for field in &schema.fields {
            let item = match &field.key {
                MsgpackKey::String(key) => obj.get(key),
                MsgpackKey::Int(idx) => obj.get(&idx.to_string()),
            };
            if let Some(item) = item {
                if !item.is_null() {
                    out.insert(field.name.clone(), restore_json(&field.ty, item, registry)?);
                }
            }
        }
        return Ok(JsonValue::Object(out));
    }
    Ok(value.clone())
}

fn restore_array(
    schema: &Schema,
    value: &JsonValue,
    registry: &Registry,
) -> Result<JsonValue, AppError> {
    let Some(items) = &schema.items else {
        return Ok(value.clone());
    };
    let Some(arr) = value.as_array() else {
        return Ok(value.clone());
    };
    arr.iter()
        .map(|item| restore_json(items, item, registry))
        .collect::<Result<Vec<_>, _>>()
        .map(JsonValue::Array)
}

fn restore_map(
    schema: &Schema,
    value: &JsonValue,
    registry: &Registry,
) -> Result<JsonValue, AppError> {
    let Some(values) = &schema.values else {
        return Ok(value.clone());
    };
    let Some(obj) = value.as_object() else {
        return Ok(value.clone());
    };
    let mut out = JsonMap::new();
    for (key, item) in obj {
        out.insert(key.clone(), restore_json(values, item, registry)?);
    }
    Ok(JsonValue::Object(out))
}

fn restore_union(
    schema: &Schema,
    value: &JsonValue,
    registry: &Registry,
) -> Result<JsonValue, AppError> {
    if value.is_null() {
        return Ok(JsonValue::Null);
    }
    let Some(variant) = schema
        .union_of
        .iter()
        .map(|s| resolve_schema(s, registry))
        .find(|s| !matches!(s.kind, SchemaKind::Null))
    else {
        return Ok(value.clone());
    };
    restore_json(&variant, value, registry)
}

fn restore_union_dispatch(
    schema: &Schema,
    value: &JsonValue,
    registry: &Registry,
) -> Result<JsonValue, AppError> {
    let Some(arr) = value.as_array() else {
        return Ok(value.clone());
    };
    if arr.len() != 2 {
        return Ok(value.clone());
    }
    let discriminator = arr[0].as_i64().unwrap_or_default();
    let variant = schema
        .union_dispatch
        .iter()
        .find(|variant| variant.key == discriminator)
        .and_then(|variant| registry.get(&variant.ty));
    let payload = if let Some(variant) = variant {
        restore_json(variant, &arr[1], registry)?
    } else {
        arr[1].clone()
    };
    Ok(serde_json::json!({
        "__type": discriminator,
        "value": payload
    }))
}

fn restore_selector(
    value: &mut JsonValue,
    selector: &str,
    schema: &Arc<Schema>,
    registry: &Registry,
) -> Result<(), AppError> {
    let segments = selector
        .split('.')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    restore_selector_segments(value, &segments, schema, registry)
}

fn restore_selector_segments(
    value: &mut JsonValue,
    segments: &[&str],
    schema: &Arc<Schema>,
    registry: &Registry,
) -> Result<(), AppError> {
    let Some((segment, rest)) = segments.split_first() else {
        *value = restore_json(schema, value, registry)?;
        return Ok(());
    };
    let (field_name, iter_array) = segment
        .strip_suffix("[]")
        .map(|name| (name, true))
        .unwrap_or((*segment, false));

    if field_name.is_empty() {
        if let Some(arr) = value.as_array_mut() {
            for item in arr {
                restore_selector_segments(item, rest, schema, registry)?;
            }
        }
        return Ok(());
    }

    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(child) = obj.get_mut(field_name) else {
        return Ok(());
    };
    if iter_array {
        if let Some(arr) = child.as_array_mut() {
            for item in arr {
                restore_selector_segments(item, rest, schema, registry)?;
            }
        }
    } else {
        restore_selector_segments(child, rest, schema, registry)?;
    }
    Ok(())
}

fn resolve_schema(schema: &Arc<Schema>, registry: &Registry) -> Arc<Schema> {
    if matches!(schema.kind, SchemaKind::Ref) {
        if let Some(name) = &schema.name {
            if let Some(real) = registry.get(name) {
                return real.clone();
            }
        }
    }
    schema.clone()
}

fn read_msgpack_json(data: &[u8]) -> Result<JsonValue, AppError> {
    let mut cursor = Cursor::new(data);
    let value = rmpv::decode::read_value(&mut cursor)
        .map_err(|e| AppError::CryptoError(format!("MsgPack decode error: {}", e)))?;
    rmpv_to_json(value)
}

fn rmpv_to_json(value: rmpv::Value) -> Result<JsonValue, AppError> {
    match value {
        rmpv::Value::Nil => Ok(JsonValue::Null),
        rmpv::Value::Boolean(b) => Ok(JsonValue::Bool(b)),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Ok(JsonValue::Number(n.into()))
            } else if let Some(n) = i.as_u64() {
                Ok(JsonValue::Number(n.into()))
            } else {
                Ok(JsonValue::Null)
            }
        }
        rmpv::Value::F32(f) => Number::from_f64(f as f64)
            .map(JsonValue::Number)
            .ok_or_else(|| AppError::CryptoError("Invalid float".to_string())),
        rmpv::Value::F64(f) => Number::from_f64(f)
            .map(JsonValue::Number)
            .ok_or_else(|| AppError::CryptoError("Invalid float".to_string())),
        rmpv::Value::String(s) => Ok(JsonValue::String(s.into_str().unwrap_or_default())),
        rmpv::Value::Binary(b) => {
            use base64::Engine as _;
            Ok(JsonValue::String(
                base64::engine::general_purpose::STANDARD.encode(&b),
            ))
        }
        rmpv::Value::Array(arr) => arr
            .into_iter()
            .map(rmpv_to_json)
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::Array),
        rmpv::Value::Map(map) => {
            let mut out = JsonMap::new();
            for (k, v) in map {
                let key = match k {
                    rmpv::Value::String(s) => s.into_str().unwrap_or_default(),
                    rmpv::Value::Integer(i) => i.to_string(),
                    _ => continue,
                };
                out.insert(key, rmpv_to_json(v)?);
            }
            Ok(JsonValue::Object(out))
        }
        rmpv::Value::Ext(_, data) => {
            use base64::Engine as _;
            Ok(JsonValue::String(
                base64::engine::general_purpose::STANDARD.encode(&data),
            ))
        }
    }
}

fn any_schema() -> Schema {
    Schema {
        kind: SchemaKind::Any,
        name: None,
        fields: Vec::new(),
        items: None,
        values: None,
        union_of: Vec::new(),
        union_dispatch: Vec::new(),
    }
}

fn null_schema() -> Schema {
    Schema {
        kind: SchemaKind::Null,
        ..any_schema()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bundle(schemas: Vec<JsonValue>, master: HashMap<String, String>) -> NuverseSchemaStore {
        NuverseSchemaStore::from_bundle(NuverseSchemaBundle {
            schemas,
            master,
            api: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn restores_int_keyed_record() {
        let schema = json!({
            "type": "record",
            "name": "UserHonor",
            "fields": [
                {"name": "honorId", "type": "int", "msgpack_key": 0},
                {"name": "level", "type": "int", "msgpack_key": 1},
                {"name": "obtainedAt", "type": "long", "msgpack_key": 2}
            ]
        });
        let store = bundle(vec![schema], HashMap::new());
        let value = json!([1001, 2, 1711000000_i64]);
        let restored =
            restore_json(store.schema("UserHonor").unwrap(), &value, &store.registry).unwrap();
        assert_eq!(restored["honorId"], json!(1001));
        assert_eq!(restored["level"], json!(2));
        assert_eq!(restored["obtainedAt"], json!(1711000000_i64));
    }

    #[test]
    fn restores_nested_array() {
        let schema = json!({
            "type": "record",
            "name": "Card",
            "fields": [
                {"name": "id", "type": "int", "msgpack_key": 0},
                {"name": "costs", "msgpack_key": 1, "type": {
                    "type": "array",
                    "items": {
                        "type": "record",
                        "name": "Cost",
                        "fields": [
                            {"name": "resourceId", "type": "int", "msgpack_key": 0},
                            {"name": "quantity", "type": "int", "msgpack_key": 1}
                        ]
                    }
                }}
            ]
        });
        let store = bundle(vec![schema], HashMap::new());
        let value = json!([1, [[100, 10], [200, 20]]]);
        let restored =
            restore_json(store.schema("Card").unwrap(), &value, &store.registry).unwrap();
        assert_eq!(restored["costs"][0]["resourceId"], json!(100));
        assert_eq!(restored["costs"][1]["quantity"], json!(20));
    }

    #[test]
    fn restores_union_dispatch() {
        let schemas = json!([
            {"type":"record","name":"A","fields":[{"name":"x","type":"int","msgpack_key":0}]},
            {"type":"record","name":"UnionBase","fields":[],"msgpack_unions":[{"key":0,"type":"A"}]}
        ]);
        let store = bundle(vec![schemas], HashMap::new());
        let value = json!([0, [42]]);
        let restored =
            restore_json(store.schema("UnionBase").unwrap(), &value, &store.registry).unwrap();
        assert_eq!(restored["__type"], json!(0));
        assert_eq!(restored["value"]["x"], json!(42));
    }

    #[test]
    fn restores_api_field_selector() {
        let schema = json!({
            "type": "record",
            "name": "UserCard",
            "fields": [
                {"name": "cardId", "type": "int", "msgpack_key": 0},
                {"name": "level", "type": "int", "msgpack_key": 1}
            ]
        });
        let store = NuverseSchemaStore::from_bundle(NuverseSchemaBundle {
            schemas: vec![schema],
            master: HashMap::new(),
            api: vec![ApiSchemaMapping {
                path: "/event/{eventId}/ranking-border".to_string(),
                schema: None,
                fields: vec![ApiFieldMapping {
                    selector: "borderRankings[].userCard".to_string(),
                    schema: "UserCard".to_string(),
                }],
            }],
        })
        .unwrap();
        let value = json!({"borderRankings":[{"userCard":[100,30]}]});
        let restored = store
            .restore_api_json("/event/123/ranking-border", value)
            .unwrap();
        assert_eq!(
            restored["borderRankings"][0]["userCard"]["cardId"],
            json!(100)
        );
        assert_eq!(
            restored["borderRankings"][0]["userCard"]["level"],
            json!(30)
        );
    }

    #[test]
    fn loads_generated_dummy_dll_bundle() {
        let data = std::fs::read("Data/structures/nuverse_schema_bundle.json").unwrap();
        let store = NuverseSchemaStore::from_slice(&data).unwrap();

        let api_value = json!({"rankings":[{"userCard":[100,30,1,2,3,4,5,0,"done","normal",0,1711000000_i64,[[1,"read",["ok"],true]]]}]});
        let api_restored = store
            .restore_api_json(
                "/user/{userId}/event/123/ranking?rankingViewType=top100",
                api_value,
            )
            .unwrap();
        assert_eq!(
            api_restored["rankings"][0]["userCard"]["cardId"],
            json!(100)
        );
        assert_eq!(
            api_restored["rankings"][0]["userCard"]["episodes"][0]["cardEpisodeId"],
            json!(1)
        );

        let master = json!({
            "actionSets": [[1, 2, "scenario", false, null, "script", null, [1, 2], null, null, 0]]
        });
        let msgpack = rmp_serde::to_vec(&master).unwrap();
        let restored = store.restore_master_msgpack(&msgpack).unwrap();
        assert_eq!(restored["actionSets"][0]["id"], json!(1));
        assert_eq!(restored["actionSets"][0]["characterIds"], json!([1, 2]));
    }
}

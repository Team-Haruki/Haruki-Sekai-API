use indexmap::IndexMap;
use serde_json::{json, Value as JsonValue};

use crate::error::AppError;

pub fn restore_dict(
    array_data: &[JsonValue],
    key_structure: &[JsonValue],
) -> IndexMap<String, JsonValue> {
    let mut result = IndexMap::new();
    for (i, key) in key_structure.iter().enumerate() {
        if i >= array_data.len() {
            break;
        }
        let value = &array_data[i];
        if value.is_null() {
            continue;
        }
        if let Some(key_str) = key.as_str() {
            result.insert(key_str.to_string(), value.clone());
        } else if let Some(key_arr) = key.as_array() {
            if key_arr.len() >= 2 {
                let field_name = key_arr[0].as_str().unwrap_or("");
                let sub_structure = &key_arr[1];
                if let Some(sub_arr) = sub_structure.as_array() {
                    if let Some(value_arr) = value.as_array() {
                        let restored: Vec<JsonValue> = value_arr
                            .iter()
                            .filter(|sub| !sub.is_null())
                            .map(|sub_item| {
                                if let Some(arr) = sub_item.as_array() {
                                    json!(restore_dict(arr, sub_arr))
                                } else {
                                    sub_item.clone()
                                }
                            })
                            .collect();
                        result.insert(field_name.to_string(), json!(restored));
                    }
                } else if let Some(sub_obj) = sub_structure.as_object() {
                    if let Some(tuple_keys) = sub_obj.get("__tuple__").and_then(|v| v.as_array()) {
                        if let Some(value_arr) = value.as_array() {
                            let mut dict = IndexMap::new();
                            for (idx, v) in value_arr.iter().enumerate() {
                                if !v.is_null() {
                                    if let Some(key_name) =
                                        tuple_keys.get(idx).and_then(|k| k.as_str())
                                    {
                                        dict.insert(key_name.to_string(), v.clone());
                                    }
                                }
                            }
                            result.insert(field_name.to_string(), json!(dict));
                        }
                    }
                }
            }
        }
    }
    result
}

/// Restores `userHonors` inside a profile response from Nuverse servers (TW/KR/CN).
/// These servers may return each `userHonors` element as a flat array `[honorId, level, obtainedAt]`
/// instead of a keyed dict. If elements are already dicts, they are left unchanged.
pub fn restore_profile_user_honors(data: &mut JsonValue) {
    let structure: Vec<JsonValue> = vec![json!("honorId"), json!("level"), json!("obtainedAt")];

    if let Some(honors) = data.get_mut("userHonors") {
        if let Some(arr) = honors.as_array_mut() {
            for item in arr.iter_mut() {
                if item.is_array() {
                    if let Some(flat) = item.as_array() {
                        let restored = restore_dict(flat, &structure);
                        *item = json!(restored);
                    }
                }
            }
        }
    }
}

/// Restores `userCard` fields inside event ranking responses from Nuverse servers (TW/KR/CN).
/// These servers return `userCard` as a flat array instead of a keyed dictionary.
/// If `userCard` is already a dict, it is left unchanged.
///
/// Handles all ranking response variants:
/// - Top100: `rankings[]`, `userWorldBloomChapterRankings[].rankings[]`
/// - Border: `borderRankings[]`, `userWorldBloomChapterRankingBorders[].borderRankings[]`
pub fn restore_ranking_user_cards(data: &mut JsonValue) {
    let user_card_structure: Vec<JsonValue> = vec![
        json!("cardId"),
        json!("level"),
        json!("exp"),
        json!("totalExp"),
        json!("skillLevel"),
        json!("skillExp"),
        json!("totalSkillExp"),
        json!("masterRank"),
        json!("specialTrainingStatus"),
        json!("defaultImage"),
        json!("duplicateCount"),
        json!("createdAt"),
        json!([
            "episodes",
            [
                "cardEpisodeId",
                "scenarioStatus",
                "scenarioStatusReasons",
                "isNotSkipped"
            ]
        ]),
    ];

    // Top-level: "rankings" (top100) and "borderRankings" (border)
    for key in &["rankings", "borderRankings"] {
        if let Some(arr) = data.get_mut(*key).and_then(|v| v.as_array_mut()) {
            restore_user_cards_in_array(arr, &user_card_structure);
        }
    }

    // Nested: userWorldBloomChapterRankings[].rankings
    if let Some(chapters) = data
        .get_mut("userWorldBloomChapterRankings")
        .and_then(|v| v.as_array_mut())
    {
        for chapter in chapters.iter_mut() {
            if let Some(arr) = chapter.get_mut("rankings").and_then(|v| v.as_array_mut()) {
                restore_user_cards_in_array(arr, &user_card_structure);
            }
        }
    }

    // Nested: userWorldBloomChapterRankingBorders[].borderRankings
    if let Some(chapters) = data
        .get_mut("userWorldBloomChapterRankingBorders")
        .and_then(|v| v.as_array_mut())
    {
        for chapter in chapters.iter_mut() {
            if let Some(arr) = chapter
                .get_mut("borderRankings")
                .and_then(|v| v.as_array_mut())
            {
                restore_user_cards_in_array(arr, &user_card_structure);
            }
        }
    }
}

/// Restores `userCard` from flat array to keyed dict for each entry in a rankings array.
fn restore_user_cards_in_array(entries: &mut [JsonValue], structure: &[JsonValue]) {
    for entry in entries.iter_mut() {
        if let Some(user_card) = entry.get("userCard") {
            if user_card.is_array() {
                if let Some(arr) = user_card.as_array() {
                    let restored = restore_dict(arr, structure);
                    entry
                        .as_object_mut()
                        .unwrap()
                        .insert("userCard".to_string(), json!(restored));
                }
            }
        }
    }
}

pub fn restore_compact_data(
    data: &IndexMap<String, JsonValue>,
) -> Vec<IndexMap<String, JsonValue>> {
    let enum_def = data.get("__ENUM__").and_then(|v| v.as_object());
    let mut column_labels: Vec<&String> = Vec::new();
    let mut columns: Vec<Vec<JsonValue>> = Vec::new();
    for (column, values) in data.iter() {
        if column == "__ENUM__" {
            continue;
        }
        column_labels.push(column);
        if let Some(arr) = values.as_array() {
            if let Some(enums) = enum_def {
                if let Some(enum_values) = enums.get(column).and_then(|v| v.as_array()) {
                    let mapped: Vec<JsonValue> = arr
                        .iter()
                        .map(|idx| {
                            if idx.is_null() {
                                JsonValue::Null
                            } else if let Some(i) = idx.as_u64() {
                                enum_values
                                    .get(i as usize)
                                    .cloned()
                                    .unwrap_or(JsonValue::Null)
                            } else {
                                idx.clone()
                            }
                        })
                        .collect();
                    columns.push(mapped);
                    continue;
                }
            }
            columns.push(arr.clone());
        }
    }
    if columns.is_empty() {
        return Vec::new();
    }
    let num_entries = columns.iter().map(|c| c.len()).min().unwrap_or(0);
    let mut result = Vec::with_capacity(num_entries);
    for i in 0..num_entries {
        let mut entry = IndexMap::new();
        for (label, column) in column_labels.iter().zip(columns.iter()) {
            entry.insert((*label).clone(), column[i].clone());
        }
        result.push(entry);
    }
    result
}

pub fn nuverse_master_restorer(
    master_data: &IndexMap<String, JsonValue>,
    structures: &IndexMap<String, JsonValue>,
) -> Result<IndexMap<String, JsonValue>, AppError> {
    let mut restored_compact_master: IndexMap<String, JsonValue> = IndexMap::new();
    let mut processed_data: IndexMap<String, JsonValue> = IndexMap::new();
    let mut restored_from_compact: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for (key, value) in master_data.iter() {
        if let Some(new_key_original) = key.strip_prefix("compact") {
            restored_compact_master.insert(key.clone(), value.clone());
            let data_map: Option<IndexMap<String, JsonValue>> = value
                .as_object()
                .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
            if let Some(data) = data_map {
                let restored = restore_compact_data(&data);
                if let Some(first_char) = new_key_original.chars().next() {
                    let new_key = format!(
                        "{}{}",
                        first_char.to_lowercase(),
                        &new_key_original[first_char.len_utf8()..]
                    );
                    restored_compact_master.insert(new_key.clone(), json!(restored));
                    restored_from_compact.insert(new_key);
                }
            }
            continue;
        }
        if restored_from_compact.contains(key) {
            continue;
        }
        let id_key = if key == "eventCards" {
            Some("cardId")
        } else {
            None
        };
        if let Some(structure) = structures.get(key) {
            if let (Some(arr), Some(struct_arr)) = (value.as_array(), structure.as_array()) {
                let restored: Vec<JsonValue> = arr
                    .iter()
                    .filter_map(|item| item.as_array().map(|a| json!(restore_dict(a, struct_arr))))
                    .collect();
                processed_data.insert(key.clone(), json!(restored));
            } else {
                processed_data.insert(key.clone(), value.clone());
            }
        } else {
            processed_data.insert(key.clone(), value.clone());
        }
        if let Some(id_k) = id_key {
            if let Some(processed_arr) = processed_data.get(key).and_then(|v| v.as_array()) {
                if let Some(value_arr) = value.as_array() {
                    let existing_ids: std::collections::HashSet<_> = processed_arr
                        .iter()
                        .filter_map(|item| item.get(id_k).and_then(|v| v.as_i64()))
                        .collect();
                    let mut merged: Vec<JsonValue> = value_arr
                        .iter()
                        .filter(|x| {
                            x.get(id_k)
                                .and_then(|v| v.as_i64())
                                .map(|id| !existing_ids.contains(&id))
                                .unwrap_or(true)
                        })
                        .cloned()
                        .collect();
                    merged.extend(processed_arr.iter().cloned());
                    merged.sort_by(|a, b| {
                        let a_id = a.get(id_k).and_then(|v| v.as_i64()).unwrap_or(0);
                        let b_id = b.get(id_k).and_then(|v| v.as_i64()).unwrap_or(0);
                        a_id.cmp(&b_id)
                    });
                    processed_data.insert(key.clone(), json!(merged));
                }
            }
        }
    }
    for (k, v) in processed_data {
        if !restored_compact_master.contains_key(&k) {
            restored_compact_master.insert(k, v);
        }
    }
    Ok(restored_compact_master)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restore_profile_user_honors_flat() {
        let mut data = json!({
            "userHonors": [
                [1001, 2, 1711000000_i64],
                [1002, 1, 1712000000_i64]
            ]
        });
        restore_profile_user_honors(&mut data);
        let honors = data["userHonors"].as_array().unwrap();
        assert_eq!(honors[0]["honorId"], json!(1001));
        assert_eq!(honors[0]["level"], json!(2));
        assert_eq!(honors[0]["obtainedAt"], json!(1711000000_i64));
        assert_eq!(honors[1]["honorId"], json!(1002));
    }

    #[test]
    fn test_restore_profile_user_honors_already_dict() {
        // Already a dict — should be left unchanged
        let mut data = json!({
            "userHonors": [
                {"honorId": 1001, "level": 2, "obtainedAt": 1711000000_i64}
            ]
        });
        restore_profile_user_honors(&mut data);
        let honors = data["userHonors"].as_array().unwrap();
        assert_eq!(honors[0]["honorId"], json!(1001));
    }

    #[test]
    fn test_restore_dict_simple() {
        let array_data = vec![json!(1), json!("hello"), json!(true)];
        let structure = vec![json!("id"), json!("name"), json!("active")];
        let result = restore_dict(&array_data, &structure);
        assert_eq!(result.get("id"), Some(&json!(1)));
        assert_eq!(result.get("name"), Some(&json!("hello")));
        assert_eq!(result.get("active"), Some(&json!(true)));
    }

    #[test]
    fn test_restore_dict_nested() {
        let array_data = vec![json!(1), json!([[100, 10], [200, 20]])];
        let structure = vec![json!("id"), json!(["costs", ["resourceId", "quantity"]])];
        let result = restore_dict(&array_data, &structure);
        assert_eq!(result.get("id"), Some(&json!(1)));
        let costs = result.get("costs").unwrap().as_array().unwrap();
        assert_eq!(costs.len(), 2);
        assert_eq!(costs[0].get("resourceId"), Some(&json!(100)));
        assert_eq!(costs[0].get("quantity"), Some(&json!(10)));
    }

    #[test]
    fn test_restore_dict_tuple() {
        let array_data = vec![json!(1), json!([100, 10])];
        let structure = vec![
            json!("id"),
            json!(["cost", {"__tuple__": ["resourceId", "quantity"]}]),
        ];
        let result = restore_dict(&array_data, &structure);
        assert_eq!(result.get("id"), Some(&json!(1)));
        let cost = result.get("cost").unwrap();
        assert_eq!(cost.get("resourceId"), Some(&json!(100)));
        assert_eq!(cost.get("quantity"), Some(&json!(10)));
    }

    #[test]
    fn test_restore_compact_data() {
        let mut data = IndexMap::new();
        data.insert("id".to_string(), json!([1, 2, 3]));
        data.insert("name".to_string(), json!(["a", "b", "c"]));
        let result = restore_compact_data(&data);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].get("id"), Some(&json!(1)));
        assert_eq!(result[0].get("name"), Some(&json!("a")));
    }

    #[test]
    fn test_restore_compact_data_with_enum() {
        let mut data = IndexMap::new();
        data.insert("id".to_string(), json!([1, 2]));
        data.insert("status".to_string(), json!([0, 1]));
        data.insert(
            "__ENUM__".to_string(),
            json!({
                "status": ["inactive", "active"]
            }),
        );
        let result = restore_compact_data(&data);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].get("status"), Some(&json!("inactive")));
        assert_eq!(result[1].get("status"), Some(&json!("active")));
    }
}

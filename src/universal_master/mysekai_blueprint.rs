//! UniversalMysekaiBlueprint - Merged mysekai blueprint data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaiblueprints::MysekaiblueprintElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaiblueprintElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiBlueprint {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_craft_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub craft_target_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_enable_sketch: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_obtained_by_convert: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub craft_count_limit: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_available_without_possession: Option<bool>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiBlueprint {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaiblueprintElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiBlueprint {
            id,
            mysekai_craft_type: get_first_value(regional, |m| {
                m.mysekai_craft_type.as_ref().map(|t| format!("{:?}", t))
            }),
            craft_target_id: get_first_value(regional, |m| m.craft_target_id),
            is_enable_sketch: get_first_value(regional, |m| m.is_enable_sketch),
            is_obtained_by_convert: get_first_value(regional, |m| m.is_obtained_by_convert),
            craft_count_limit: get_first_value(regional, |m| m.craft_count_limit.flatten()),
            is_available_without_possession: get_first_value(regional, |m| {
                m.is_available_without_possession.flatten()
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_blueprints(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaiblueprintElement>>,
) -> Vec<UniversalMysekaiBlueprint> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiBlueprint> = by_id
        .values()
        .filter_map(UniversalMysekaiBlueprint::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

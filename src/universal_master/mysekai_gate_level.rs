//! UniversalMysekaiGateLevel - Merged gate level data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaigatelevels::MysekaigatelevelElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaigatelevelElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiGateLevel {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_gate_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_gate_material_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_gate_character_visit_count_rate_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_bonus_rate: Option<f64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiGateLevel {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaigatelevelElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiGateLevel {
            id,
            mysekai_gate_id: get_first_value(regional, |m| m.mysekai_gate_id),
            level: get_first_value(regional, |m| m.level),
            mysekai_gate_material_group_id: get_first_value(regional, |m| {
                m.mysekai_gate_material_group_id
            }),
            mysekai_gate_character_visit_count_rate_id: get_first_value(regional, |m| {
                m.mysekai_gate_character_visit_count_rate_id
            }),
            power_bonus_rate: get_first_value(regional, |m| m.power_bonus_rate),
            available_regions,
        })
    }
}

pub fn merge_mysekai_gate_levels(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaigatelevelElement>>,
) -> Vec<UniversalMysekaiGateLevel> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiGateLevel> = by_id
        .values()
        .filter_map(UniversalMysekaiGateLevel::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

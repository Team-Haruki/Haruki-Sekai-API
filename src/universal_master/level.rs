//! UniversalLevel - Merged level/exp data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::levels::LevelElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for LevelElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.flatten().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalLevel {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub level_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_exp: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalLevel {
    pub fn from_regional(regional: &super::types::RegionalData<LevelElement>) -> Option<Self> {
        let id = get_first_value(regional, |l| l.id.flatten())?;
        let available_regions = regional.available_regions();

        Some(UniversalLevel {
            id,
            level_type: get_first_value(regional, |l| {
                l.level_type.as_ref().map(|t| format!("{:?}", t))
            }),
            level: get_first_value(regional, |l| l.level),
            total_exp: get_first_value(regional, |l| l.total_exp),
            available_regions,
        })
    }
}

pub fn merge_levels(
    region_data: std::collections::HashMap<ServerRegion, Vec<LevelElement>>,
) -> Vec<UniversalLevel> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalLevel> = by_id
        .values()
        .filter_map(UniversalLevel::from_regional)
        .collect();
    result.sort_by_key(|l| l.id);
    result
}

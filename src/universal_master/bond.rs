//! UniversalBond - Merged bond data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::bonds::BondElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for BondElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalBond {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id1: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id2: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalBond {
    pub fn from_regional(regional: &super::types::RegionalData<BondElement>) -> Option<Self> {
        let id = get_first_value(regional, |b| b.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalBond {
            id,
            group_id: get_first_value(regional, |b| b.group_id),
            character_id1: get_first_value(regional, |b| b.character_id1),
            character_id2: get_first_value(regional, |b| b.character_id2),
            available_regions,
        })
    }
}

pub fn merge_bonds(
    region_data: std::collections::HashMap<ServerRegion, Vec<BondElement>>,
) -> Vec<UniversalBond> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalBond> = by_id
        .values()
        .filter_map(UniversalBond::from_regional)
        .collect();
    result.sort_by_key(|b| b.id);
    result
}

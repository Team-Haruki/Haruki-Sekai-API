//! UniversalMysekaiGateMaterialGroup - Merged gate material group data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaigatematerialgroups::MysekaigatematerialgroupElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaigatematerialgroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiGateMaterialGroup {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_material_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiGateMaterialGroup {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaigatematerialgroupElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiGateMaterialGroup {
            id,
            group_id: get_first_value(regional, |m| m.group_id),
            mysekai_material_id: get_first_value(regional, |m| m.mysekai_material_id),
            quantity: get_first_value(regional, |m| m.quantity),
            available_regions,
        })
    }
}

pub fn merge_mysekai_gate_material_groups(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaigatematerialgroupElement>>,
) -> Vec<UniversalMysekaiGateMaterialGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiGateMaterialGroup> = by_id
        .values()
        .filter_map(UniversalMysekaiGateMaterialGroup::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

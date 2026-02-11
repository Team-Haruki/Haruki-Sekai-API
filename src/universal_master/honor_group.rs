//! UniversalHonorGroup - Merged honor group data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::honorgroups::HonorgroupElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for HonorgroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalHonorGroup {
    pub id: i64,

    // Universal fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub honor_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_name: Option<String>,

    // Regional fields
    pub name: UnifiedValue<String>,

    pub pronunciation: UnifiedValue<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalHonorGroup {
    pub fn from_regional(regional: &super::types::RegionalData<HonorgroupElement>) -> Option<Self> {
        let id = get_first_value(regional, |h| h.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let honor_type = get_first_value(regional, |h| {
            h.honor_type.as_ref().map(|t| format!("{:?}", t))
        });
        let background_assetbundle_name = get_first_value(regional, |h| {
            h.background_assetbundle_name.clone().flatten()
        });
        let frame_name = get_first_value(regional, |h| h.frame_name.clone().flatten());

        // Regional fields
        let name = merge_field(regional, |h| h.name.clone());
        let pronunciation = merge_field(regional, |h| h.pronunciation.clone());

        Some(UniversalHonorGroup {
            id,
            honor_type,
            background_assetbundle_name,
            frame_name,
            name,
            pronunciation,
            available_regions,
        })
    }
}

pub fn merge_honor_groups(
    region_data: std::collections::HashMap<ServerRegion, Vec<HonorgroupElement>>,
) -> Vec<UniversalHonorGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalHonorGroup> = by_id
        .values()
        .filter_map(UniversalHonorGroup::from_regional)
        .collect();
    result.sort_by_key(|h| h.id);
    result
}

//! UniversalPlayerFrameGroup - Mixed: name is regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::playerframegroups::PlayerframegroupElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for PlayerframegroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalPlayerFrameGroup {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalPlayerFrameGroup {
    pub fn from_regional(
        regional: &super::types::RegionalData<PlayerframegroupElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |p| p.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalPlayerFrameGroup {
            id,
            seq: get_first_value(regional, |p| p.seq),
            name: merge_field(regional, |p| p.name.clone()),
            assetbundle_name: get_first_value(regional, |p| p.assetbundle_name.clone()),
            available_regions,
        })
    }
}

pub fn merge_player_frame_groups(
    region_data: std::collections::HashMap<ServerRegion, Vec<PlayerframegroupElement>>,
) -> Vec<UniversalPlayerFrameGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalPlayerFrameGroup> = by_id
        .values()
        .filter_map(UniversalPlayerFrameGroup::from_regional)
        .collect();
    result.sort_by_key(|p| p.id);
    result
}

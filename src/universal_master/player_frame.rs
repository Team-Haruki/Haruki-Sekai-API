//! UniversalPlayerFrame - Mixed: description is regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::playerframes::PlayerframeElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for PlayerframeElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalPlayerFrame {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_frame_group_id: Option<i64>,

    pub description: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalPlayerFrame {
    pub fn from_regional(
        regional: &super::types::RegionalData<PlayerframeElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |p| p.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalPlayerFrame {
            id,
            seq: get_first_value(regional, |p| p.seq),
            player_frame_group_id: get_first_value(regional, |p| p.player_frame_group_id),
            description: merge_field(regional, |p| p.description.clone()),
            game_character_id: get_first_value(regional, |p| p.game_character_id),
            available_regions,
        })
    }
}

pub fn merge_player_frames(
    region_data: std::collections::HashMap<ServerRegion, Vec<PlayerframeElement>>,
) -> Vec<UniversalPlayerFrame> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalPlayerFrame> = by_id
        .values()
        .filter_map(UniversalPlayerFrame::from_regional)
        .collect();
    result.sort_by_key(|p| p.id);
    result
}

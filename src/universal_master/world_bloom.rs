//! UniversalWorldBloom - Mixed: timestamps are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::worldblooms::WorldbloomElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for WorldbloomElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalWorldBloom {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_bloom_chapter_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapter_no: Option<i64>,

    pub chapter_start_at: UnifiedValue<i64>,

    pub aggregate_at: UnifiedValue<i64>,

    pub chapter_end_at: UnifiedValue<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_supplemental: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub costume2d_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalWorldBloom {
    pub fn from_regional(regional: &super::types::RegionalData<WorldbloomElement>) -> Option<Self> {
        let id = get_first_value(regional, |w| w.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalWorldBloom {
            id,
            event_id: get_first_value(regional, |w| w.event_id),
            game_character_id: get_first_value(regional, |w| w.game_character_id.flatten()),
            world_bloom_chapter_type: get_first_value(regional, |w| {
                w.world_bloom_chapter_type
                    .as_ref()
                    .and_then(|t| t.as_ref().map(|v| format!("{:?}", v)))
            }),
            chapter_no: get_first_value(regional, |w| w.chapter_no),
            chapter_start_at: merge_field(regional, |w| w.chapter_start_at),
            aggregate_at: merge_field(regional, |w| w.aggregate_at),
            chapter_end_at: merge_field(regional, |w| w.chapter_end_at),
            is_supplemental: get_first_value(regional, |w| w.is_supplemental),
            costume2d_id: get_first_value(regional, |w| w.costume2_d_id.flatten()),
            available_regions,
        })
    }
}

pub fn merge_world_blooms(
    region_data: std::collections::HashMap<ServerRegion, Vec<WorldbloomElement>>,
) -> Vec<UniversalWorldBloom> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalWorldBloom> = by_id
        .values()
        .filter_map(UniversalWorldBloom::from_regional)
        .collect();
    result.sort_by_key(|w| w.id);
    result
}

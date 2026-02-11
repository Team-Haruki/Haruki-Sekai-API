//! UniversalEventStory - Merged event story data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventstories::{EventStoryEpisode, EventstorieElement};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for EventstorieElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventStory {
    pub id: i64,

    // Universal fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner_game_character_unit_id: Option<i64>,

    /// Episodes — treated as JP-only (like cardParameters), structure/rewards identical
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_story_episodes: Option<Vec<EventStoryEpisode>>,

    // Regional fields
    /// Localized story synopsis
    pub outline: UnifiedValue<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventStory {
    pub fn from_regional(
        regional: &super::types::RegionalData<EventstorieElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |s| s.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let event_id = get_first_value(regional, |s| s.event_id);
        let assetbundle_name = get_first_value(regional, |s| s.assetbundle_name.clone());
        let banner_game_character_unit_id =
            get_first_value(regional, |s| s.banner_game_character_unit_id.flatten());

        // Episodes — take JP value (structure identical, only title differs)
        let event_story_episodes = get_first_value(regional, |s| s.event_story_episodes.clone());

        // Regional fields
        let outline = merge_field(regional, |s| s.outline.clone());

        Some(UniversalEventStory {
            id,
            event_id,
            assetbundle_name,
            banner_game_character_unit_id,
            event_story_episodes,
            outline,
            available_regions,
        })
    }
}

pub fn merge_event_stories(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventstorieElement>>,
) -> Vec<UniversalEventStory> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventStory> = by_id
        .values()
        .filter_map(UniversalEventStory::from_regional)
        .collect();
    result.sort_by_key(|s| s.id);
    result
}

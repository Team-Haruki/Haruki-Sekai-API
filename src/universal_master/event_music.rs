//! UniversalEventMusic - Merged event music links across all regions
//! Fully universal — no regional differences. Uses composite key (event_id, music_id).

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventmusics::EventmusicElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for EventmusicElement {
    type Id = (i64, i64);
    fn id(&self) -> Self::Id {
        (self.event_id.unwrap_or(0), self.music_id.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventMusic {
    pub event_id: i64,

    pub music_id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_condition_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventMusic {
    pub fn from_regional(regional: &super::types::RegionalData<EventmusicElement>) -> Option<Self> {
        let event_id = get_first_value(regional, |e| e.event_id)?;
        let music_id = get_first_value(regional, |e| e.music_id)?;
        let available_regions = regional.available_regions();

        Some(UniversalEventMusic {
            event_id,
            music_id,
            seq: get_first_value(regional, |e| e.seq),
            release_condition_id: get_first_value(regional, |e| e.release_condition_id),
            available_regions,
        })
    }
}

pub fn merge_event_musics(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventmusicElement>>,
) -> Vec<UniversalEventMusic> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventMusic> = by_id
        .values()
        .filter_map(UniversalEventMusic::from_regional)
        .collect();
    result.sort_by_key(|e| (e.event_id, e.music_id));
    result
}

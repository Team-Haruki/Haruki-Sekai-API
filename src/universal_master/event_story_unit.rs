//! UniversalEventStoryUnit - Merged event story unit data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventstoryunits::EventstoryunitElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for EventstoryunitElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventStoryUnit {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_story_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_story_unit_relation: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventStoryUnit {
    pub fn from_regional(
        regional: &super::types::RegionalData<EventstoryunitElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |e| e.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalEventStoryUnit {
            id,
            seq: get_first_value(regional, |e| e.seq),
            event_story_id: get_first_value(regional, |e| e.event_story_id),
            unit: get_first_value(regional, |e| e.unit.as_ref().map(|u| format!("{:?}", u))),
            event_story_unit_relation: get_first_value(regional, |e| {
                e.event_story_unit_relation
                    .as_ref()
                    .map(|r| format!("{:?}", r))
            }),
            available_regions,
        })
    }
}

pub fn merge_event_story_units(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventstoryunitElement>>,
) -> Vec<UniversalEventStoryUnit> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventStoryUnit> = by_id
        .values()
        .filter_map(UniversalEventStoryUnit::from_regional)
        .collect();
    result.sort_by_key(|e| e.id);
    result
}

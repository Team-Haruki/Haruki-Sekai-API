//! UniversalEventCard - Merged event card bonus data across all regions
//! Fully universal — no regional differences. Uses composite key (card_id, event_id).

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventcards::EventcardElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for EventcardElement {
    type Id = (i64, i64);
    fn id(&self) -> Self::Id {
        (self.card_id.unwrap_or(0), self.event_id.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventCard {
    pub card_id: i64,

    pub event_id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub leader_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_display_card_story: Option<bool>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventCard {
    pub fn from_regional(regional: &super::types::RegionalData<EventcardElement>) -> Option<Self> {
        let card_id = get_first_value(regional, |e| e.card_id)?;
        let event_id = get_first_value(regional, |e| e.event_id)?;
        let available_regions = regional.available_regions();

        Some(UniversalEventCard {
            card_id,
            event_id,
            bonus_rate: get_first_value(regional, |e| e.bonus_rate),
            leader_bonus_rate: get_first_value(regional, |e| e.leader_bonus_rate.flatten()),
            is_display_card_story: get_first_value(regional, |e| e.is_display_card_story),
            available_regions,
        })
    }
}

pub fn merge_event_cards(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventcardElement>>,
) -> Vec<UniversalEventCard> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventCard> = by_id
        .values()
        .filter_map(UniversalEventCard::from_regional)
        .collect();
    result.sort_by_key(|e| (e.event_id, e.card_id));
    result
}

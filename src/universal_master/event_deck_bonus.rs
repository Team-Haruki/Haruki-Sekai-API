//! UniversalEventDeckBonus - Merged event deck bonus data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventdeckbonuses::EventdeckbonuseElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for EventdeckbonuseElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventDeckBonus {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_attr: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonus_rate: Option<f64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventDeckBonus {
    pub fn from_regional(
        regional: &super::types::RegionalData<EventdeckbonuseElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |e| e.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalEventDeckBonus {
            id,
            event_id: get_first_value(regional, |e| e.event_id),
            game_character_unit_id: get_first_value(regional, |e| {
                e.game_character_unit_id.flatten()
            }),
            card_attr: get_first_value(regional, |e| {
                e.card_attr
                    .as_ref()
                    .and_then(|a| a.as_ref().map(|v| format!("{:?}", v)))
            }),
            bonus_rate: get_first_value(regional, |e| e.bonus_rate),
            available_regions,
        })
    }
}

pub fn merge_event_deck_bonuses(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventdeckbonuseElement>>,
) -> Vec<UniversalEventDeckBonus> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventDeckBonus> = by_id
        .values()
        .filter_map(UniversalEventDeckBonus::from_regional)
        .collect();
    result.sort_by_key(|e| e.id);
    result
}

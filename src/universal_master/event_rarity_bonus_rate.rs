//! UniversalEventRarityBonusRate - Merged event rarity bonus rate data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventraritybonusrates::EventraritybonusrateElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for EventraritybonusrateElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventRarityBonusRate {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_rarity_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_rank: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonus_rate: Option<f64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventRarityBonusRate {
    pub fn from_regional(
        regional: &super::types::RegionalData<EventraritybonusrateElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |e| e.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalEventRarityBonusRate {
            id,
            card_rarity_type: get_first_value(regional, |e| {
                e.card_rarity_type.as_ref().map(|t| format!("{:?}", t))
            }),
            master_rank: get_first_value(regional, |e| e.master_rank),
            bonus_rate: get_first_value(regional, |e| e.bonus_rate),
            available_regions,
        })
    }
}

pub fn merge_event_rarity_bonus_rates(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventraritybonusrateElement>>,
) -> Vec<UniversalEventRarityBonusRate> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventRarityBonusRate> = by_id
        .values()
        .filter_map(UniversalEventRarityBonusRate::from_regional)
        .collect();
    result.sort_by_key(|e| e.id);
    result
}

//! UniversalCardMysekaiCanvasBonus - Merged card mysekai canvas bonus data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::cardmysekaicanvasbonuses::CardmysekaicanvasbonuseElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for CardmysekaicanvasbonuseElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCardMysekaiCanvasBonus {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_rarity_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power1_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power2_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power3_bonus_fixed: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCardMysekaiCanvasBonus {
    pub fn from_regional(
        regional: &super::types::RegionalData<CardmysekaicanvasbonuseElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCardMysekaiCanvasBonus {
            id,
            card_rarity_type: get_first_value(regional, |c| c.card_rarity_type.clone()),
            power1_bonus_fixed: get_first_value(regional, |c| c.power1_bonus_fixed),
            power2_bonus_fixed: get_first_value(regional, |c| c.power2_bonus_fixed),
            power3_bonus_fixed: get_first_value(regional, |c| c.power3_bonus_fixed),
            available_regions,
        })
    }
}

pub fn merge_card_mysekai_canvas_bonuses(
    region_data: std::collections::HashMap<ServerRegion, Vec<CardmysekaicanvasbonuseElement>>,
) -> Vec<UniversalCardMysekaiCanvasBonus> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCardMysekaiCanvasBonus> = by_id
        .values()
        .filter_map(UniversalCardMysekaiCanvasBonus::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

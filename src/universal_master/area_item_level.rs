//! UniversalAreaItemLevel - Mixed: sentence is regional, composite key (area_item_id, level)
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::areaitemlevels::AreaitemlevelElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for AreaitemlevelElement {
    type Id = (i64, i64);
    fn id(&self) -> Self::Id {
        (self.area_item_id.unwrap_or(0), self.level.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalAreaItemLevel {
    pub area_item_id: i64,

    pub level: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_unit: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_card_attr: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_game_character_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power1_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power1_all_match_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power2_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power2_all_match_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power3_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power3_all_match_bonus_rate: Option<f64>,

    pub sentence: UnifiedValue<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalAreaItemLevel {
    pub fn from_regional(
        regional: &super::types::RegionalData<AreaitemlevelElement>,
    ) -> Option<Self> {
        let area_item_id = get_first_value(regional, |a| a.area_item_id)?;
        let level = get_first_value(regional, |a| a.level)?;
        let available_regions = regional.available_regions();

        Some(UniversalAreaItemLevel {
            area_item_id,
            level,
            target_unit: get_first_value(regional, |a| {
                a.target_unit.as_ref().map(|t| format!("{:?}", t))
            }),
            target_card_attr: get_first_value(regional, |a| {
                a.target_card_attr.as_ref().map(|t| format!("{:?}", t))
            }),
            target_game_character_id: get_first_value(regional, |a| {
                a.target_game_character_id.flatten()
            }),
            power1_bonus_rate: get_first_value(regional, |a| a.power1_bonus_rate),
            power1_all_match_bonus_rate: get_first_value(regional, |a| {
                a.power1_all_match_bonus_rate
            }),
            power2_bonus_rate: get_first_value(regional, |a| a.power2_bonus_rate),
            power2_all_match_bonus_rate: get_first_value(regional, |a| {
                a.power2_all_match_bonus_rate
            }),
            power3_bonus_rate: get_first_value(regional, |a| a.power3_bonus_rate),
            power3_all_match_bonus_rate: get_first_value(regional, |a| {
                a.power3_all_match_bonus_rate
            }),
            sentence: merge_field(regional, |a| a.sentence.clone()),
            available_regions,
        })
    }
}

pub fn merge_area_item_levels(
    region_data: std::collections::HashMap<ServerRegion, Vec<AreaitemlevelElement>>,
) -> Vec<UniversalAreaItemLevel> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalAreaItemLevel> = by_id
        .values()
        .filter_map(UniversalAreaItemLevel::from_regional)
        .collect();
    result.sort_by(|a, b| {
        a.area_item_id
            .cmp(&b.area_item_id)
            .then(a.level.cmp(&b.level))
    });
    result
}

//! UniversalGameCharacterUnit - Merged game character unit data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::gamecharacterunits::GamecharacterunitElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for GamecharacterunitElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalGameCharacterUnit {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_code: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin_color_code: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin_shadow_color_code1: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin_shadow_color_code2: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalGameCharacterUnit {
    pub fn from_regional(
        regional: &super::types::RegionalData<GamecharacterunitElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |g| g.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalGameCharacterUnit {
            id,
            game_character_id: get_first_value(regional, |g| g.game_character_id),
            unit: get_first_value(regional, |g| g.unit.as_ref().map(|u| format!("{:?}", u))),
            color_code: get_first_value(regional, |g| g.color_code.clone()),
            skin_color_code: get_first_value(regional, |g| {
                g.skin_color_code.as_ref().map(|s| format!("{:?}", s))
            }),
            skin_shadow_color_code1: get_first_value(regional, |g| {
                g.skin_shadow_color_code1
                    .as_ref()
                    .map(|s| format!("{:?}", s))
            }),
            skin_shadow_color_code2: get_first_value(regional, |g| {
                g.skin_shadow_color_code2
                    .as_ref()
                    .map(|s| format!("{:?}", s))
            }),
            available_regions,
        })
    }
}

pub fn merge_game_character_units(
    region_data: std::collections::HashMap<ServerRegion, Vec<GamecharacterunitElement>>,
) -> Vec<UniversalGameCharacterUnit> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalGameCharacterUnit> = by_id
        .values()
        .filter_map(UniversalGameCharacterUnit::from_regional)
        .collect();
    result.sort_by_key(|g| g.id);
    result
}

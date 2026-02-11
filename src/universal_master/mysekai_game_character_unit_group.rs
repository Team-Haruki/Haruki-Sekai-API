//! UniversalMysekaiGameCharacterUnitGroup
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaigamecharacterunitgroups::MysekaigamecharacterunitgroupElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaigamecharacterunitgroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiGameCharacterUnitGroup {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id1: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id2: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id3: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id4: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id5: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiGameCharacterUnitGroup {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaigamecharacterunitgroupElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiGameCharacterUnitGroup {
            id,
            game_character_unit_id1: get_first_value(regional, |m| m.game_character_unit_id1),
            game_character_unit_id2: get_first_value(regional, |m| {
                m.game_character_unit_id2.flatten()
            }),
            game_character_unit_id3: get_first_value(regional, |m| {
                m.game_character_unit_id3.flatten()
            }),
            game_character_unit_id4: get_first_value(regional, |m| {
                m.game_character_unit_id4.flatten()
            }),
            game_character_unit_id5: get_first_value(regional, |m| {
                m.game_character_unit_id5.flatten()
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_game_character_unit_groups(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaigamecharacterunitgroupElement>>,
) -> Vec<UniversalMysekaiGameCharacterUnitGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiGameCharacterUnitGroup> = by_id
        .values()
        .filter_map(UniversalMysekaiGameCharacterUnitGroup::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

//! UniversalMysekaiCharacterTalk - Merged mysekai character talk data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaicharactertalks::MysekaicharactertalkElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaicharactertalkElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiCharacterTalk {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_game_character_unit_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_condition_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_site_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_term_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_archive_mysekai_character_talk_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lua: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_enabled_for_multi: Option<bool>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiCharacterTalk {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaicharactertalkElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiCharacterTalk {
            id,
            mysekai_game_character_unit_group_id: get_first_value(regional, |m| {
                m.mysekai_game_character_unit_group_id
            }),
            mysekai_character_talk_condition_group_id: get_first_value(regional, |m| {
                m.mysekai_character_talk_condition_group_id
            }),
            mysekai_site_group_id: get_first_value(regional, |m| m.mysekai_site_group_id),
            mysekai_character_talk_term_id: get_first_value(regional, |m| {
                m.mysekai_character_talk_term_id
            }),
            character_archive_mysekai_character_talk_group_id: get_first_value(regional, |m| {
                m.character_archive_mysekai_character_talk_group_id
            }),
            assetbundle_name: get_first_value(regional, |m| {
                m.assetbundle_name.as_ref().map(|a| format!("{:?}", a))
            }),
            lua: get_first_value(regional, |m| m.lua.clone()),
            is_enabled_for_multi: get_first_value(regional, |m| m.is_enabled_for_multi),
            available_regions,
        })
    }
}

pub fn merge_mysekai_character_talks(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaicharactertalkElement>>,
) -> Vec<UniversalMysekaiCharacterTalk> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiCharacterTalk> = by_id
        .values()
        .filter_map(UniversalMysekaiCharacterTalk::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

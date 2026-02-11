//! UniversalMysekaiCharacterTalkFixtureCommon - Merged fixture common data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaicharactertalkfixturecommons::MysekaicharactertalkfixturecommonElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaicharactertalkfixturecommonElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiCharacterTalkFixtureCommon {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_fixture_common_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_fixture_common_mysekai_fixture_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_fixture_common_tweet_group_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiCharacterTalkFixtureCommon {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaicharactertalkfixturecommonElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiCharacterTalkFixtureCommon {
            id,
            game_character_unit_id: get_first_value(regional, |m| m.game_character_unit_id),
            mysekai_character_talk_fixture_common_type: get_first_value(regional, |m| {
                m.mysekai_character_talk_fixture_common_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            mysekai_character_talk_fixture_common_mysekai_fixture_group_id: get_first_value(
                regional,
                |m| m.mysekai_character_talk_fixture_common_mysekai_fixture_group_id,
            ),
            mysekai_character_talk_fixture_common_tweet_group_id: get_first_value(regional, |m| {
                m.mysekai_character_talk_fixture_common_tweet_group_id
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_character_talk_fixture_commons(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<MysekaicharactertalkfixturecommonElement>,
    >,
) -> Vec<UniversalMysekaiCharacterTalkFixtureCommon> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiCharacterTalkFixtureCommon> = by_id
        .values()
        .filter_map(UniversalMysekaiCharacterTalkFixtureCommon::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

//! UniversalMysekaiCharacterTalkCondition - Merged condition data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaicharactertalkconditions::MysekaicharactertalkconditionElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaicharactertalkconditionElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiCharacterTalkCondition {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_condition_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_condition_type_value: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiCharacterTalkCondition {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaicharactertalkconditionElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiCharacterTalkCondition {
            id,
            mysekai_character_talk_condition_type: get_first_value(regional, |m| {
                m.mysekai_character_talk_condition_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            mysekai_character_talk_condition_type_value: get_first_value(regional, |m| {
                m.mysekai_character_talk_condition_type_value
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_character_talk_conditions(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaicharactertalkconditionElement>>,
) -> Vec<UniversalMysekaiCharacterTalkCondition> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiCharacterTalkCondition> = by_id
        .values()
        .filter_map(UniversalMysekaiCharacterTalkCondition::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

//! UniversalMysekaiCharacterTalkConditionGroup - Merged condition group data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaicharactertalkconditiongroups::MysekaicharactertalkconditiongroupElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaicharactertalkconditiongroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiCharacterTalkConditionGroup {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_character_talk_condition_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiCharacterTalkConditionGroup {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaicharactertalkconditiongroupElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiCharacterTalkConditionGroup {
            id,
            group_id: get_first_value(regional, |m| m.group_id),
            mysekai_character_talk_condition_id: get_first_value(regional, |m| {
                m.mysekai_character_talk_condition_id
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_character_talk_condition_groups(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<MysekaicharactertalkconditiongroupElement>,
    >,
) -> Vec<UniversalMysekaiCharacterTalkConditionGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiCharacterTalkConditionGroup> = by_id
        .values()
        .filter_map(UniversalMysekaiCharacterTalkConditionGroup::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

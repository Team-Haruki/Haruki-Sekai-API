//! UniversalMysekaiCharacterTalkFixtureCommonFixtureGroup - Merged fixture group mapping
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaicharactertalkfixturecommonmysekaifixturegroups::MysekaicharactertalkfixturecommonmysekaifixturegroupElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaicharactertalkfixturecommonmysekaifixturegroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiCharacterTalkFixtureCommonFixtureGroup {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_fixture_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiCharacterTalkFixtureCommonFixtureGroup {
    pub fn from_regional(
        regional: &super::types::RegionalData<
            MysekaicharactertalkfixturecommonmysekaifixturegroupElement,
        >,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiCharacterTalkFixtureCommonFixtureGroup {
            id,
            group_id: get_first_value(regional, |m| m.group_id),
            mysekai_fixture_id: get_first_value(regional, |m| m.mysekai_fixture_id),
            available_regions,
        })
    }
}

pub fn merge_mysekai_character_talk_fixture_common_fixture_groups(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<MysekaicharactertalkfixturecommonmysekaifixturegroupElement>,
    >,
) -> Vec<UniversalMysekaiCharacterTalkFixtureCommonFixtureGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiCharacterTalkFixtureCommonFixtureGroup> = by_id
        .values()
        .filter_map(UniversalMysekaiCharacterTalkFixtureCommonFixtureGroup::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

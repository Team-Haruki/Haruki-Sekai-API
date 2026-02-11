//! UniversalCharacterArchiveMysekaiCharacterTalkGroup
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::characterarchivemysekaicharactertalkgroups::CharacterarchivemysekaicharactertalkgroupElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for CharacterarchivemysekaicharactertalkgroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCharacterArchiveMysekaiCharacterTalkGroup {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_display_type: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCharacterArchiveMysekaiCharacterTalkGroup {
    pub fn from_regional(
        regional: &super::types::RegionalData<CharacterarchivemysekaicharactertalkgroupElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCharacterArchiveMysekaiCharacterTalkGroup {
            id,
            seq: get_first_value(regional, |c| c.seq),
            archive_display_type: get_first_value(regional, |c| {
                c.archive_display_type.as_ref().map(|t| format!("{:?}", t))
            }),
            available_regions,
        })
    }
}

pub fn merge_character_archive_mysekai_character_talk_groups(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<CharacterarchivemysekaicharactertalkgroupElement>,
    >,
) -> Vec<UniversalCharacterArchiveMysekaiCharacterTalkGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCharacterArchiveMysekaiCharacterTalkGroup> = by_id
        .values()
        .filter_map(UniversalCharacterArchiveMysekaiCharacterTalkGroup::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

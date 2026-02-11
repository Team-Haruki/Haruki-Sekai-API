//! UniversalCharacterRank - Merged character rank data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::characterranks::{CharacterRankAchieveResource, CharacterrankElement};

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for CharacterrankElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCharacterRank {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_rank: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power1_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power2_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power3_bonus_rate: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reward_resource_box_ids: Option<Vec<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_rank_achieve_resources: Option<Vec<CharacterRankAchieveResource>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCharacterRank {
    pub fn from_regional(
        regional: &super::types::RegionalData<CharacterrankElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCharacterRank {
            id,
            character_id: get_first_value(regional, |c| c.character_id),
            character_rank: get_first_value(regional, |c| c.character_rank),
            power1_bonus_rate: get_first_value(regional, |c| c.power1_bonus_rate),
            power2_bonus_rate: get_first_value(regional, |c| c.power2_bonus_rate),
            power3_bonus_rate: get_first_value(regional, |c| c.power3_bonus_rate),
            reward_resource_box_ids: get_first_value(regional, |c| {
                c.reward_resource_box_ids.clone()
            }),
            character_rank_achieve_resources: get_first_value(regional, |c| {
                c.character_rank_achieve_resources.clone()
            }),
            available_regions,
        })
    }
}

pub fn merge_character_ranks(
    region_data: std::collections::HashMap<ServerRegion, Vec<CharacterrankElement>>,
) -> Vec<UniversalCharacterRank> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCharacterRank> = by_id
        .values()
        .filter_map(UniversalCharacterRank::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

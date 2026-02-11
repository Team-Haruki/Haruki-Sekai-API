//! UniversalChallengeLiveHighScoreReward - Merged challenge live reward data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::challengelivehighscorerewards::ChallengelivehighscorerewardElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for ChallengelivehighscorerewardElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalChallengeLiveHighScoreReward {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_score: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_box_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalChallengeLiveHighScoreReward {
    pub fn from_regional(
        regional: &super::types::RegionalData<ChallengelivehighscorerewardElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalChallengeLiveHighScoreReward {
            id,
            character_id: get_first_value(regional, |c| c.character_id),
            high_score: get_first_value(regional, |c| c.high_score),
            resource_box_id: get_first_value(regional, |c| c.resource_box_id),
            available_regions,
        })
    }
}

pub fn merge_challenge_live_high_score_rewards(
    region_data: std::collections::HashMap<ServerRegion, Vec<ChallengelivehighscorerewardElement>>,
) -> Vec<UniversalChallengeLiveHighScoreReward> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalChallengeLiveHighScoreReward> = by_id
        .values()
        .filter_map(UniversalChallengeLiveHighScoreReward::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

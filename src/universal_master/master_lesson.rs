//! UniversalMasterLesson - Merged master lesson data across all regions
//! Fully universal — no regional differences. Uses composite key (card_rarity_type, master_rank).

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::masterlessons::{Cost, MasterlessonElement};

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MasterlessonElement {
    type Id = (String, i64);
    fn id(&self) -> Self::Id {
        (
            self.card_rarity_type
                .as_ref()
                .map(|t| format!("{:?}", t))
                .unwrap_or_default(),
            self.master_rank.unwrap_or(0),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMasterLesson {
    pub card_rarity_type: String,

    pub master_rank: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power1_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power2_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power3_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_rank_exp: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub costs: Option<Vec<Cost>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewards: Option<Vec<Option<serde_json::Value>>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMasterLesson {
    pub fn from_regional(
        regional: &super::types::RegionalData<MasterlessonElement>,
    ) -> Option<Self> {
        let card_rarity_type = get_first_value(regional, |m| {
            m.card_rarity_type.as_ref().map(|t| format!("{:?}", t))
        })?;
        let master_rank = get_first_value(regional, |m| m.master_rank)?;
        let available_regions = regional.available_regions();

        Some(UniversalMasterLesson {
            card_rarity_type,
            master_rank,
            power1_bonus_fixed: get_first_value(regional, |m| m.power1_bonus_fixed),
            power2_bonus_fixed: get_first_value(regional, |m| m.power2_bonus_fixed),
            power3_bonus_fixed: get_first_value(regional, |m| m.power3_bonus_fixed),
            character_rank_exp: get_first_value(regional, |m| m.character_rank_exp.flatten()),
            costs: get_first_value(regional, |m| m.costs.clone()),
            rewards: get_first_value(regional, |m| m.rewards.clone().flatten()),
            available_regions,
        })
    }
}

pub fn merge_master_lessons(
    region_data: std::collections::HashMap<ServerRegion, Vec<MasterlessonElement>>,
) -> Vec<UniversalMasterLesson> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMasterLesson> = by_id
        .values()
        .filter_map(UniversalMasterLesson::from_regional)
        .collect();
    result.sort_by(|a, b| {
        a.card_rarity_type
            .cmp(&b.card_rarity_type)
            .then(a.master_rank.cmp(&b.master_rank))
    });
    result
}

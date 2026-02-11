//! UniversalCardEpisode - Merged card episode data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::cardepisodes::{CardepisodeElement, Cost};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for CardepisodeElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCardEpisode {
    pub id: i64,

    // Universal fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_episode_part_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_condition_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power1_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power2_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub power3_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reward_resource_box_ids: Option<Vec<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub costs: Option<Vec<Cost>>,

    // Regional fields
    /// Localized episode title (JP: "サイドストーリー（前編）" vs CN: "卡牌剧情（上篇）")
    pub title: UnifiedValue<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCardEpisode {
    pub fn from_regional(
        regional: &super::types::RegionalData<CardepisodeElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |e| e.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let card_id = get_first_value(regional, |e| e.card_id);
        let scenario_id = get_first_value(regional, |e| e.scenario_id.clone());
        let card_episode_part_type = get_first_value(regional, |e| {
            e.card_episode_part_type
                .as_ref()
                .map(|t| format!("{:?}", t))
        });
        let assetbundle_name = get_first_value(regional, |e| e.assetbundle_name.clone().flatten());
        let release_condition_id = get_first_value(regional, |e| e.release_condition_id);
        let power1_bonus_fixed = get_first_value(regional, |e| e.power1_bonus_fixed);
        let power2_bonus_fixed = get_first_value(regional, |e| e.power2_bonus_fixed);
        let power3_bonus_fixed = get_first_value(regional, |e| e.power3_bonus_fixed);
        let reward_resource_box_ids =
            get_first_value(regional, |e| e.reward_resource_box_ids.clone().flatten());
        let costs = get_first_value(regional, |e| e.costs.clone());

        // Regional fields
        let title = merge_field(regional, |e| e.title.clone());

        Some(UniversalCardEpisode {
            id,
            card_id,
            scenario_id,
            card_episode_part_type,
            assetbundle_name,
            release_condition_id,
            power1_bonus_fixed,
            power2_bonus_fixed,
            power3_bonus_fixed,
            reward_resource_box_ids,
            costs,
            title,
            available_regions,
        })
    }
}

pub fn merge_card_episodes(
    region_data: std::collections::HashMap<ServerRegion, Vec<CardepisodeElement>>,
) -> Vec<UniversalCardEpisode> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCardEpisode> = by_id
        .values()
        .filter_map(UniversalCardEpisode::from_regional)
        .collect();
    result.sort_by_key(|e| e.id);
    result
}

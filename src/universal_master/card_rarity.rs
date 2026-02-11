//! UniversalCardRarity - Merged card rarity data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::cardrarities::CardraritieElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for CardraritieElement {
    type Id = String;
    fn id(&self) -> Self::Id {
        self.card_rarity_type.clone().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCardRarity {
    pub card_rarity_type: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_level: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_skill_level: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub training_max_level: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCardRarity {
    pub fn from_regional(
        regional: &super::types::RegionalData<CardraritieElement>,
    ) -> Option<Self> {
        let card_rarity_type = get_first_value(regional, |r| r.card_rarity_type.clone())?;
        let available_regions = regional.available_regions();

        Some(UniversalCardRarity {
            card_rarity_type,
            seq: get_first_value(regional, |r| r.seq),
            max_level: get_first_value(regional, |r| r.max_level),
            max_skill_level: get_first_value(regional, |r| r.max_skill_level),
            training_max_level: get_first_value(regional, |r| r.training_max_level.flatten()),
            available_regions,
        })
    }
}

pub fn merge_card_rarities(
    region_data: std::collections::HashMap<ServerRegion, Vec<CardraritieElement>>,
) -> Vec<UniversalCardRarity> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCardRarity> = by_id
        .values()
        .filter_map(UniversalCardRarity::from_regional)
        .collect();
    result.sort_by(|a, b| a.card_rarity_type.cmp(&b.card_rarity_type));
    result
}

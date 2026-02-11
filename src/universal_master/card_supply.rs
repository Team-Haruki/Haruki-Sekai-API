//! UniversalCardSupply - Merged card supply data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::cardsupplies::CardsupplieElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for CardsupplieElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCardSupply {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_supply_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCardSupply {
    pub fn from_regional(
        regional: &super::types::RegionalData<CardsupplieElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |s| s.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCardSupply {
            id,
            card_supply_type: get_first_value(regional, |s| s.card_supply_type.clone()),
            assetbundle_name: get_first_value(regional, |s| s.assetbundle_name.clone().flatten()),
            available_regions,
        })
    }
}

pub fn merge_card_supplies(
    region_data: std::collections::HashMap<ServerRegion, Vec<CardsupplieElement>>,
) -> Vec<UniversalCardSupply> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCardSupply> = by_id
        .values()
        .filter_map(UniversalCardSupply::from_regional)
        .collect();
    result.sort_by_key(|s| s.id);
    result
}

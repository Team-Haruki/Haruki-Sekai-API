//! UniversalCardCostume3d - Merged card-costume mappings across all regions
//! Fully universal — no regional differences. Uses composite key (card_id, costume3d_id).

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::cardcostume3ds::Cardcostume3DElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for Cardcostume3DElement {
    type Id = (i64, i64);
    fn id(&self) -> Self::Id {
        (self.card_id.unwrap_or(0), self.costume3_d_id.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCardCostume3d {
    pub card_id: i64,

    #[serde(rename = "costume3dId")]
    pub costume3d_id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_initial_obtain_hair: Option<bool>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCardCostume3d {
    pub fn from_regional(
        regional: &super::types::RegionalData<Cardcostume3DElement>,
    ) -> Option<Self> {
        let card_id = get_first_value(regional, |c| c.card_id)?;
        let costume3d_id = get_first_value(regional, |c| c.costume3_d_id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCardCostume3d {
            card_id,
            costume3d_id,
            is_initial_obtain_hair: get_first_value(regional, |c| {
                c.is_initial_obtain_hair.flatten()
            }),
            available_regions,
        })
    }
}

pub fn merge_card_costume3ds(
    region_data: std::collections::HashMap<ServerRegion, Vec<Cardcostume3DElement>>,
) -> Vec<UniversalCardCostume3d> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCardCostume3d> = by_id
        .values()
        .filter_map(UniversalCardCostume3d::from_regional)
        .collect();
    result.sort_by_key(|c| (c.card_id, c.costume3d_id));
    result
}

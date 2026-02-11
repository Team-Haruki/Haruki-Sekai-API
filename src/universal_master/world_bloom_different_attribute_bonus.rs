//! RegionalWorldBloomDifferentAttributeBonus - Region-wrapped data
//! Not suitable for Universal merge — each region has independent data.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::worldbloomdifferentattributebonuses::WorldbloomdifferentattributebonuseElement;

use super::types::RegionalData;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionalWorldBloomDifferentAttributeBonuses {
    #[serde(flatten)]
    pub data: RegionalData<Vec<WorldbloomdifferentattributebonuseElement>>,
}

pub fn wrap_world_bloom_different_attribute_bonuses(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<WorldbloomdifferentattributebonuseElement>,
    >,
) -> RegionalWorldBloomDifferentAttributeBonuses {
    let mut data = RegionalData::new();
    for (region, items) in region_data {
        data.set(region, items);
    }
    RegionalWorldBloomDifferentAttributeBonuses { data }
}

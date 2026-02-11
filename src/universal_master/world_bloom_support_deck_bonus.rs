//! RegionalWorldBloomSupportDeckBonus - Region-wrapped data
//! Not suitable for Universal merge — each region has independent data.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::worldbloomsupportdeckbonuses::WorldbloomsupportdeckbonuseElement;

use super::types::RegionalData;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionalWorldBloomSupportDeckBonuses {
    #[serde(flatten)]
    pub data: RegionalData<Vec<WorldbloomsupportdeckbonuseElement>>,
}

pub fn wrap_world_bloom_support_deck_bonuses(
    region_data: std::collections::HashMap<ServerRegion, Vec<WorldbloomsupportdeckbonuseElement>>,
) -> RegionalWorldBloomSupportDeckBonuses {
    let mut data = RegionalData::new();
    for (region, items) in region_data {
        data.set(region, items);
    }
    RegionalWorldBloomSupportDeckBonuses { data }
}

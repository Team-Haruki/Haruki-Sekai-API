//! RegionalWorldBloomSupportDeckUnitEventLimitedBonus - Region-wrapped data
//! Not suitable for Universal merge — each region has independent data.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::worldbloomsupportdeckuniteventlimitedbonuses::WorldbloomsupportdeckuniteventlimitedbonuseElement;

use super::types::RegionalData;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionalWorldBloomSupportDeckUnitEventLimitedBonuses {
    #[serde(flatten)]
    pub data: RegionalData<Vec<WorldbloomsupportdeckuniteventlimitedbonuseElement>>,
}

pub fn wrap_world_bloom_support_deck_unit_event_limited_bonuses(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<WorldbloomsupportdeckuniteventlimitedbonuseElement>,
    >,
) -> RegionalWorldBloomSupportDeckUnitEventLimitedBonuses {
    let mut data = RegionalData::new();
    for (region, items) in region_data {
        data.set(region, items);
    }
    RegionalWorldBloomSupportDeckUnitEventLimitedBonuses { data }
}

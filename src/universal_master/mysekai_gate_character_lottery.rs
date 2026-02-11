//! UniversalMysekaiGateCharacterLottery - Merged gate lottery data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaigatecharacterlotteries::MysekaigatecharacterlotterieElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaigatecharacterlotterieElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiGateCharacterLottery {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_gate_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub visitable_mysekai_gate_level: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiGateCharacterLottery {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaigatecharacterlotterieElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiGateCharacterLottery {
            id,
            mysekai_gate_id: get_first_value(regional, |m| m.mysekai_gate_id),
            game_character_unit_id: get_first_value(regional, |m| m.game_character_unit_id),
            visitable_mysekai_gate_level: get_first_value(regional, |m| {
                m.visitable_mysekai_gate_level
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_gate_character_lotteries(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaigatecharacterlotterieElement>>,
) -> Vec<UniversalMysekaiGateCharacterLottery> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiGateCharacterLottery> = by_id
        .values()
        .filter_map(UniversalMysekaiGateCharacterLottery::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

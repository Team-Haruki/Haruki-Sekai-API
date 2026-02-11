//! UniversalMysekaiMaterialGameCharacterRelation
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaimaterialgamecharacterrelations::MysekaimaterialgamecharacterrelationElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaimaterialgamecharacterrelationElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiMaterialGameCharacterRelation {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_material_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiMaterialGameCharacterRelation {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaimaterialgamecharacterrelationElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiMaterialGameCharacterRelation {
            id,
            group_id: get_first_value(regional, |m| m.group_id),
            mysekai_material_id: get_first_value(regional, |m| m.mysekai_material_id),
            game_character_id: get_first_value(regional, |m| m.game_character_id),
            available_regions,
        })
    }
}

pub fn merge_mysekai_material_game_character_relations(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<MysekaimaterialgamecharacterrelationElement>,
    >,
) -> Vec<UniversalMysekaiMaterialGameCharacterRelation> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiMaterialGameCharacterRelation> = by_id
        .values()
        .filter_map(UniversalMysekaiMaterialGameCharacterRelation::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

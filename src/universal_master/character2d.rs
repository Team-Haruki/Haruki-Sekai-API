//! UniversalCharacter2d - Merged 2D character config across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::character2ds::{Character2DElement, CharacterType, Unit};

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for Character2DElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCharacter2d {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_type: Option<CharacterType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_next_grade: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_enabled_flip_display: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_icon_assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCharacter2d {
    pub fn from_regional(
        regional: &super::types::RegionalData<Character2DElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCharacter2d {
            id,
            character_type: get_first_value(regional, |c| c.character_type.clone()),
            is_next_grade: get_first_value(regional, |c| c.is_next_grade),
            character_id: get_first_value(regional, |c| c.character_id),
            unit: get_first_value(regional, |c| c.unit.clone()),
            is_enabled_flip_display: get_first_value(regional, |c| c.is_enabled_flip_display),
            asset_name: get_first_value(regional, |c| c.asset_name.clone().flatten()),
            character_icon_assetbundle_name: get_first_value(regional, |c| {
                c.character_icon_assetbundle_name
                    .as_ref()
                    .map(|v| format!("{:?}", v))
            }),
            available_regions,
        })
    }
}

pub fn merge_character2ds(
    region_data: std::collections::HashMap<ServerRegion, Vec<Character2DElement>>,
) -> Vec<UniversalCharacter2d> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCharacter2d> = by_id
        .values()
        .filter_map(UniversalCharacter2d::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

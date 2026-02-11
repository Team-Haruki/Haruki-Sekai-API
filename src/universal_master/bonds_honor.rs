//! UniversalBondsHonor - Mixed: name, pronunciation, description are regional
//! Nested levels[].description is also regional, requiring per-level merging (same pattern as honor.rs)
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::bondshonors::{BondshonorElement, Level};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::{RegionalData, UnifiedValue};

impl Mergeable for BondshonorElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

/// Merged bonds honor level with regional description
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalBondsHonorLevel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,

    /// Localized level description
    pub description: UnifiedValue<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalBondsHonor {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonds_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id1: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id2: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub honor_rarity: Option<String>,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronunciation: Option<UnifiedValue<String>>,

    pub description: UnifiedValue<String>,

    /// Merged levels with regional descriptions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<UniversalBondsHonorLevel>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub configurable_unit_virtual_singer: Option<bool>,

    pub available_regions: Vec<ServerRegion>,
}

/// Merge levels across regions: match by level number, merge descriptions
fn merge_levels(
    regional: &RegionalData<BondshonorElement>,
) -> Option<Vec<UniversalBondsHonorLevel>> {
    let mut level_map: std::collections::HashMap<i64, RegionalData<Level>> =
        std::collections::HashMap::new();

    let regions = [
        (&regional.jp, ServerRegion::Jp),
        (&regional.en, ServerRegion::En),
        (&regional.tw, ServerRegion::Tw),
        (&regional.kr, ServerRegion::Kr),
        (&regional.cn, ServerRegion::Cn),
    ];

    for (maybe_item, region) in &regions {
        if let Some(item) = maybe_item {
            if let Some(levels) = &item.levels {
                for level in levels {
                    let level_num = level.level.unwrap_or(0);
                    let entry = level_map.entry(level_num).or_insert_with(RegionalData::new);
                    entry.set(*region, level.clone());
                }
            }
        }
    }

    if level_map.is_empty() {
        return None;
    }

    let mut merged_levels: Vec<UniversalBondsHonorLevel> = level_map
        .iter()
        .map(|(_, level_regional)| {
            let id = get_first_value(level_regional, |l| l.id);
            let level = get_first_value(level_regional, |l| l.level);
            let description = merge_field(level_regional, |l| l.description.clone());

            UniversalBondsHonorLevel {
                id,
                level,
                description,
            }
        })
        .collect();

    merged_levels.sort_by_key(|l| l.level.unwrap_or(0));
    Some(merged_levels)
}

impl UniversalBondsHonor {
    pub fn from_regional(regional: &RegionalData<BondshonorElement>) -> Option<Self> {
        let id = get_first_value(regional, |b| b.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalBondsHonor {
            id,
            seq: get_first_value(regional, |b| b.seq),
            bonds_group_id: get_first_value(regional, |b| b.bonds_group_id),
            game_character_unit_id1: get_first_value(regional, |b| b.game_character_unit_id1),
            game_character_unit_id2: get_first_value(regional, |b| b.game_character_unit_id2),
            honor_rarity: get_first_value(regional, |b| {
                b.honor_rarity.as_ref().map(|r| format!("{:?}", r))
            }),
            name: merge_field(regional, |b| b.name.clone()),
            pronunciation: Some(merge_field(regional, |b| b.pronunciation.clone())),
            description: merge_field(regional, |b| b.description.clone()),
            levels: merge_levels(regional),
            configurable_unit_virtual_singer: get_first_value(regional, |b| {
                b.configurable_unit_virtual_singer
            }),
            available_regions,
        })
    }
}

pub fn merge_bonds_honors(
    region_data: std::collections::HashMap<ServerRegion, Vec<BondshonorElement>>,
) -> Vec<UniversalBondsHonor> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalBondsHonor> = by_id
        .values()
        .filter_map(UniversalBondsHonor::from_regional)
        .collect();
    result.sort_by_key(|b| b.id);
    result
}

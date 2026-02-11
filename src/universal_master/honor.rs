//! UniversalHonor - Merged honor data across all regions
//! Note: levels[].description is Regional, requiring per-level merging.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::honors::{HonorElement, Level};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::{RegionalData, UnifiedValue};

impl Mergeable for HonorElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

/// Merged honor level with regional description
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalHonorLevel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonus: Option<i64>,

    /// Localized level description
    pub description: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub honor_rarity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalHonor {
    pub id: i64,

    // Universal fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub honor_rarity: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub honor_type_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub honor_mission_type: Option<String>,

    /// Merged levels with regional descriptions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<UniversalHonorLevel>>,

    // Regional fields
    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_at: Option<UnifiedValue<i64>>,

    pub available_regions: Vec<ServerRegion>,
}

/// Merge levels across regions: match by level number, merge descriptions
fn merge_levels(regional: &RegionalData<HonorElement>) -> Option<Vec<UniversalHonorLevel>> {
    // Collect all levels from all regions, keyed by level number
    let mut level_map: std::collections::HashMap<i64, RegionalData<Level>> =
        std::collections::HashMap::new();

    let regions = [
        (&regional.jp, ServerRegion::Jp),
        (&regional.en, ServerRegion::En),
        (&regional.tw, ServerRegion::Tw),
        (&regional.kr, ServerRegion::Kr),
        (&regional.cn, ServerRegion::Cn),
    ];

    for (maybe_honor, region) in &regions {
        if let Some(honor) = maybe_honor {
            if let Some(levels) = &honor.levels {
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

    let mut merged_levels: Vec<UniversalHonorLevel> = level_map
        .iter()
        .map(|(_, level_regional)| {
            let level = get_first_value(level_regional, |l| l.level);
            let bonus = get_first_value(level_regional, |l| l.bonus);
            let description = merge_field(level_regional, |l| l.description.clone());
            let assetbundle_name = get_first_value(level_regional, |l| {
                l.assetbundle_name.clone().and_then(|v| v)
            });
            let honor_rarity = get_first_value(level_regional, |l| {
                l.honor_rarity.as_ref().map(|r| format!("{:?}", r))
            });

            UniversalHonorLevel {
                level,
                bonus,
                description,
                assetbundle_name,
                honor_rarity,
            }
        })
        .collect();

    merged_levels.sort_by_key(|l| l.level.unwrap_or(0));
    Some(merged_levels)
}

impl UniversalHonor {
    pub fn from_regional(regional: &RegionalData<HonorElement>) -> Option<Self> {
        let id = get_first_value(regional, |h| h.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let group_id = get_first_value(regional, |h| h.group_id);
        let honor_rarity = get_first_value(regional, |h| {
            h.honor_rarity.as_ref().map(|r| format!("{:?}", r))
        });
        let assetbundle_name =
            get_first_value(regional, |h| h.assetbundle_name.clone().and_then(|v| v));
        let honor_type_id = get_first_value(regional, |h| h.honor_type_id.flatten());
        let honor_mission_type =
            get_first_value(regional, |h| h.honor_mission_type.clone().and_then(|v| v));

        // Merged levels with regional descriptions
        let levels = merge_levels(regional);

        // Regional fields
        let name = merge_field(regional, |h| h.name.clone());
        let start_at = {
            let v = merge_field(regional, |h| h.start_at.flatten());
            Some(v)
        };

        Some(UniversalHonor {
            id,
            group_id,
            honor_rarity,
            assetbundle_name,
            honor_type_id,
            honor_mission_type,
            levels,
            name,
            start_at,
            available_regions,
        })
    }
}

pub fn merge_honors(
    region_data: std::collections::HashMap<ServerRegion, Vec<HonorElement>>,
) -> Vec<UniversalHonor> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalHonor> = by_id
        .values()
        .filter_map(UniversalHonor::from_regional)
        .collect();
    result.sort_by_key(|h| h.id);
    result
}

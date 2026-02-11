//! UniversalStamp - Merged stamp data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::stamps::StampElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for StampElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalStamp {
    pub id: i64,

    // Universal fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stamp_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub balloon_assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id1: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id2: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_unit_id: Option<i64>,

    // Regional fields
    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_published_at: Option<UnifiedValue<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_display_type: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalStamp {
    pub fn from_regional(regional: &super::types::RegionalData<StampElement>) -> Option<Self> {
        let id = get_first_value(regional, |s| s.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let stamp_type = get_first_value(regional, |s| {
            s.stamp_type.as_ref().map(|t| format!("{:?}", t))
        });
        let assetbundle_name = get_first_value(regional, |s| s.assetbundle_name.clone());
        let balloon_assetbundle_name = get_first_value(regional, |s| {
            s.balloon_assetbundle_name
                .as_ref()
                .map(|b| format!("{:?}", b))
        });
        let character_id1 = get_first_value(regional, |s| s.character_id1.flatten());
        let character_id2 = get_first_value(regional, |s| s.character_id2.flatten());
        let game_character_unit_id =
            get_first_value(regional, |s| s.game_character_unit_id.flatten());

        // Regional fields
        let name = merge_field(regional, |s| s.name.clone());
        let description = {
            let v = merge_field(regional, |s| s.description.clone().and_then(|v| v));
            Some(v)
        };
        let archive_published_at = {
            let v = merge_field(regional, |s| s.archive_published_at);
            Some(v)
        };
        let archive_display_type = get_first_value(regional, |s| {
            s.archive_display_type.as_ref().map(|a| format!("{:?}", a))
        });

        Some(UniversalStamp {
            id,
            stamp_type,
            assetbundle_name,
            balloon_assetbundle_name,
            character_id1,
            character_id2,
            game_character_unit_id,
            name,
            description,
            archive_published_at,
            archive_display_type,
            available_regions,
        })
    }
}

pub fn merge_stamps(
    region_data: std::collections::HashMap<ServerRegion, Vec<StampElement>>,
) -> Vec<UniversalStamp> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalStamp> = by_id
        .values()
        .filter_map(UniversalStamp::from_regional)
        .collect();
    result.sort_by_key(|s| s.id);
    result
}

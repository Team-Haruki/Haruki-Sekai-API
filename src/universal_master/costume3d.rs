//! UniversalCostume3d - Merged 3D costume data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::costume3ds::Costume3DElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for Costume3DElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCostume3d {
    pub id: i64,

    // Universal fields
    #[serde(rename = "costume3dGroupId", skip_serializing_if = "Option::is_none")]
    pub costume3d_group_id: Option<i64>,

    #[serde(rename = "costume3dType", skip_serializing_if = "Option::is_none")]
    pub costume3d_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id: Option<i64>,

    #[serde(rename = "costume3dRarity", skip_serializing_if = "Option::is_none")]
    pub costume3d_rarity: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub designer: Option<String>,

    // Regional fields
    pub name: UnifiedValue<String>,

    pub color_name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub how_to_obtain: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_published_at: Option<UnifiedValue<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<UnifiedValue<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_display_type: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCostume3d {
    pub fn from_regional(regional: &super::types::RegionalData<Costume3DElement>) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let costume3d_group_id =
            get_first_value(regional, |c| Some(c.costume3_d_group_id)).flatten();
        let costume3d_type = get_first_value(regional, |c| {
            c.costume3_d_type.as_ref().map(|t| format!("{:?}", t))
        });
        let part_type = get_first_value(regional, |c| {
            c.part_type.as_ref().map(|t| format!("{:?}", t))
        });
        let color_id = get_first_value(regional, |c| c.color_id);
        let character_id = get_first_value(regional, |c| c.character_id);
        let costume3d_rarity = get_first_value(regional, |c| {
            c.costume3_d_rarity.as_ref().map(|r| format!("{:?}", r))
        });
        let assetbundle_name = get_first_value(regional, |c| c.assetbundle_name.clone());
        let designer = get_first_value(regional, |c| c.designer.clone());

        // Regional fields
        let name = merge_field(regional, |c| c.name.clone());
        let color_name = merge_field(regional, |c| c.color_name.clone());
        let how_to_obtain = {
            let v = merge_field(regional, |c| c.how_to_obtain.clone().and_then(|v| v));
            Some(v)
        };
        let archive_published_at = {
            let v = merge_field(regional, |c| c.archive_published_at);
            Some(v)
        };
        let published_at = {
            let v = merge_field(regional, |c| c.published_at.flatten());
            Some(v)
        };
        let archive_display_type = get_first_value(regional, |c| {
            c.archive_display_type.as_ref().map(|a| format!("{:?}", a))
        });

        Some(UniversalCostume3d {
            id,
            costume3d_group_id,
            costume3d_type,
            part_type,
            color_id,
            character_id,
            costume3d_rarity,
            assetbundle_name,
            designer,
            name,
            color_name,
            how_to_obtain,
            archive_published_at,
            published_at,
            archive_display_type,
            available_regions,
        })
    }
}

pub fn merge_costume3ds(
    region_data: std::collections::HashMap<ServerRegion, Vec<Costume3DElement>>,
) -> Vec<UniversalCostume3d> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCostume3d> = by_id
        .values()
        .filter_map(UniversalCostume3d::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

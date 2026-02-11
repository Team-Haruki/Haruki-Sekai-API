//! UniversalMysekaiMaterial - Mixed: name, pronunciation, description are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaimaterials::MysekaimaterialElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for MysekaimaterialElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiMaterial {
    pub id: i64,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronunciation: Option<UnifiedValue<String>>,

    pub description: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_material_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_material_rarity_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_site_ids: Option<Vec<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_phenomena_group_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiMaterial {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaimaterialElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiMaterial {
            id,
            name: merge_field(regional, |m| m.name.clone()),
            pronunciation: Some(merge_field(regional, |m| m.pronunciation.clone())),
            description: merge_field(regional, |m| m.description.clone()),
            mysekai_material_type: get_first_value(regional, |m| {
                m.mysekai_material_type.as_ref().map(|t| format!("{:?}", t))
            }),
            seq: get_first_value(regional, |m| m.seq),
            mysekai_material_rarity_type: get_first_value(regional, |m| {
                m.mysekai_material_rarity_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            icon_assetbundle_name: get_first_value(regional, |m| m.icon_assetbundle_name.clone()),
            model_assetbundle_name: get_first_value(regional, |m| {
                m.model_assetbundle_name.clone().and_then(|v| v)
            }),
            mysekai_site_ids: get_first_value(regional, |m| m.mysekai_site_ids.clone()),
            mysekai_phenomena_group_id: get_first_value(regional, |m| {
                m.mysekai_phenomena_group_id.and_then(|v| v)
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_materials(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaimaterialElement>>,
) -> Vec<UniversalMysekaiMaterial> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiMaterial> = by_id
        .values()
        .filter_map(UniversalMysekaiMaterial::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

//! UniversalGachaCeilItem - Mixed: name and convert_start_at are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::gachaceilitems::GachaceilitemElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for GachaceilitemElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalGachaCeilItem {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_id: Option<i64>,

    pub name: UnifiedValue<String>,

    /// AssetbundleName enum serialized as String
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub convert_start_at: Option<UnifiedValue<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub convert_resource_box_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalGachaCeilItem {
    pub fn from_regional(
        regional: &super::types::RegionalData<GachaceilitemElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |g| g.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalGachaCeilItem {
            id,
            gacha_id: get_first_value(regional, |g| g.gacha_id),
            name: merge_field(regional, |g| g.name.clone()),
            assetbundle_name: get_first_value(regional, |g| {
                g.assetbundle_name.as_ref().map(|a| format!("{:?}", a))
            }),
            convert_start_at: Some(merge_field(regional, |g| g.convert_start_at)),
            convert_resource_box_id: get_first_value(regional, |g| g.convert_resource_box_id),
            available_regions,
        })
    }
}

pub fn merge_gacha_ceil_items(
    region_data: std::collections::HashMap<ServerRegion, Vec<GachaceilitemElement>>,
) -> Vec<UniversalGachaCeilItem> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalGachaCeilItem> = by_id
        .values()
        .filter_map(UniversalGachaCeilItem::from_regional)
        .collect();
    result.sort_by_key(|g| g.id);
    result
}

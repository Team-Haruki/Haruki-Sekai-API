//! UniversalMysekaiItem - Mixed: name, pronunciation, description are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaiitems::MysekaiitemElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for MysekaiitemElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiItem {
    pub id: i64,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronunciation: Option<UnifiedValue<String>>,

    pub description: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_item_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiItem {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaiitemElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiItem {
            id,
            name: merge_field(regional, |m| m.name.clone()),
            pronunciation: Some(merge_field(regional, |m| m.pronunciation.clone())),
            description: merge_field(regional, |m| m.description.clone()),
            seq: get_first_value(regional, |m| m.seq),
            mysekai_item_type: get_first_value(regional, |m| m.mysekai_item_type.clone()),
            icon_assetbundle_name: get_first_value(regional, |m| m.icon_assetbundle_name.clone()),
            available_regions,
        })
    }
}

pub fn merge_mysekai_items(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaiitemElement>>,
) -> Vec<UniversalMysekaiItem> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiItem> = by_id
        .values()
        .filter_map(UniversalMysekaiItem::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

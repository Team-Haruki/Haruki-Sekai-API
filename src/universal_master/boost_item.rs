//! UniversalBoostItem - Mixed: name, flavor_text are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::boostitems::BoostitemElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for BoostitemElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalBoostItem {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_value: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_bundle_name: Option<String>,

    /// flavor_text is an enum with localized strings in source — serialized as Debug string
    pub flavor_text: UnifiedValue<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalBoostItem {
    pub fn from_regional(regional: &super::types::RegionalData<BoostitemElement>) -> Option<Self> {
        let id = get_first_value(regional, |b| b.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalBoostItem {
            id,
            seq: get_first_value(regional, |b| b.seq.flatten()),
            name: merge_field(regional, |b| b.name.clone()),
            recovery_value: get_first_value(regional, |b| b.recovery_value),
            asset_bundle_name: get_first_value(regional, |b| {
                b.asset_bundle_name.clone().and_then(|v| v)
            }),
            flavor_text: merge_field(regional, |b| {
                b.flavor_text.as_ref().map(|f| format!("{:?}", f))
            }),
            available_regions,
        })
    }
}

pub fn merge_boost_items(
    region_data: std::collections::HashMap<ServerRegion, Vec<BoostitemElement>>,
) -> Vec<UniversalBoostItem> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalBoostItem> = by_id
        .values()
        .filter_map(UniversalBoostItem::from_regional)
        .collect();
    result.sort_by_key(|b| b.id);
    result
}

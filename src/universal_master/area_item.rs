//! UniversalAreaItem - Mixed: name, flavor_text are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::areaitems::AreaitemElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for AreaitemElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalAreaItem {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_id: Option<i64>,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub flavor_text: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_point: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalAreaItem {
    pub fn from_regional(regional: &super::types::RegionalData<AreaitemElement>) -> Option<Self> {
        let id = get_first_value(regional, |a| a.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalAreaItem {
            id,
            area_id: get_first_value(regional, |a| a.area_id),
            name: merge_field(regional, |a| a.name.clone()),
            flavor_text: {
                let field = merge_field(regional, |a| a.flavor_text.clone().and_then(|v| v));
                Some(field)
            },
            spawn_point: get_first_value(regional, |a| {
                a.spawn_point.as_ref().map(|s| format!("{:?}", s))
            }),
            assetbundle_name: get_first_value(regional, |a| a.assetbundle_name.clone()),
            available_regions,
        })
    }
}

pub fn merge_area_items(
    region_data: std::collections::HashMap<ServerRegion, Vec<AreaitemElement>>,
) -> Vec<UniversalAreaItem> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalAreaItem> = by_id
        .values()
        .filter_map(UniversalAreaItem::from_regional)
        .collect();
    result.sort_by_key(|a| a.id);
    result
}

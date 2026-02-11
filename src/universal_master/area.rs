//! UniversalArea - Mixed: name, sub_name, label, start_at, end_at are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::areas::AreaElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for AreaElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalArea {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_base_area: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_timeline_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_area_type: Option<String>,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_condition_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_name: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_at: Option<UnifiedValue<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_at: Option<UnifiedValue<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_condition_id2: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalArea {
    pub fn from_regional(regional: &super::types::RegionalData<AreaElement>) -> Option<Self> {
        let id = get_first_value(regional, |a| a.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalArea {
            id,
            assetbundle_name: get_first_value(regional, |a| a.assetbundle_name.clone()),
            group_id: get_first_value(regional, |a| a.group_id),
            is_base_area: get_first_value(regional, |a| a.is_base_area),
            area_type: get_first_value(regional, |a| {
                a.area_type.as_ref().map(|t| format!("{:?}", t))
            }),
            view_type: get_first_value(regional, |a| {
                a.view_type.as_ref().map(|t| format!("{:?}", t))
            }),
            display_timeline_type: get_first_value(regional, |a| {
                a.display_timeline_type.as_ref().map(|t| format!("{:?}", t))
            }),
            additional_area_type: get_first_value(regional, |a| {
                a.additional_area_type.as_ref().map(|t| format!("{:?}", t))
            }),
            name: merge_field(regional, |a| a.name.clone()),
            release_condition_id: get_first_value(regional, |a| a.release_condition_id),
            sub_name: Some(merge_field(regional, |a| {
                a.sub_name.clone().and_then(|v| v)
            })),
            label: Some(merge_field(regional, |a| a.label.clone().and_then(|v| v))),
            start_at: Some(merge_field(regional, |a| a.start_at.and_then(|v| v))),
            end_at: Some(merge_field(regional, |a| a.end_at.and_then(|v| v))),
            release_condition_id2: get_first_value(regional, |a| a.release_condition_id2.flatten()),
            available_regions,
        })
    }
}

pub fn merge_areas(
    region_data: std::collections::HashMap<ServerRegion, Vec<AreaElement>>,
) -> Vec<UniversalArea> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalArea> = by_id
        .values()
        .filter_map(UniversalArea::from_regional)
        .collect();
    result.sort_by_key(|a| a.id);
    result
}

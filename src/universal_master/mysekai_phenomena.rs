//! UniversalMysekaiPhenomena - Mixed: name, english_name, description are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaiphenomenas::Mysekaiphenomenon;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for Mysekaiphenomenon {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiPhenomena {
    pub id: i64,

    pub name: UnifiedValue<String>,

    pub english_name: UnifiedValue<String>,

    pub description: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_phenomena_brightness_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_phenomena_time_period_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_phenomena_background_color_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ramp_texture_assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiPhenomena {
    pub fn from_regional(regional: &super::types::RegionalData<Mysekaiphenomenon>) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiPhenomena {
            id,
            name: merge_field(regional, |m| m.name.clone()),
            english_name: merge_field(regional, |m| m.english_name.clone()),
            description: merge_field(regional, |m| m.description.clone()),
            mysekai_phenomena_brightness_type: get_first_value(regional, |m| {
                m.mysekai_phenomena_brightness_type
                    .as_ref()
                    .map(|b| format!("{:?}", b))
            }),
            mysekai_phenomena_time_period_type: get_first_value(regional, |m| {
                m.mysekai_phenomena_time_period_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            mysekai_phenomena_background_color_id: get_first_value(regional, |m| {
                m.mysekai_phenomena_background_color_id
            }),
            assetbundle_name: get_first_value(regional, |m| m.assetbundle_name.clone()),
            ramp_texture_assetbundle_name: get_first_value(regional, |m| {
                m.ramp_texture_assetbundle_name.clone()
            }),
            icon_assetbundle_name: get_first_value(regional, |m| m.icon_assetbundle_name.clone()),
            available_regions,
        })
    }
}

pub fn merge_mysekai_phenomenas(
    region_data: std::collections::HashMap<ServerRegion, Vec<Mysekaiphenomenon>>,
) -> Vec<UniversalMysekaiPhenomena> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiPhenomena> = by_id
        .values()
        .filter_map(UniversalMysekaiPhenomena::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

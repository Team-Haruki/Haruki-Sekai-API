//! UniversalMysekaiPhenomenaBackgroundColor - Merged phenomena bg color data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaiphenomenabackgroundcolors::MysekaiphenomenabackgroundcolorElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaiphenomenabackgroundcolorElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiPhenomenaBackgroundColor {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_color: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_color: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradation_color: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner_color: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ground_highlight_color: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiPhenomenaBackgroundColor {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaiphenomenabackgroundcolorElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiPhenomenaBackgroundColor {
            id,
            base_color: get_first_value(regional, |m| m.base_color.clone()),
            ground_color: get_first_value(regional, |m| m.ground_color.clone()),
            gradation_color: get_first_value(regional, |m| m.gradation_color.clone()),
            corner_color: get_first_value(regional, |m| m.corner_color.clone()),
            ground_highlight_color: get_first_value(regional, |m| m.ground_highlight_color.clone()),
            available_regions,
        })
    }
}

pub fn merge_mysekai_phenomena_background_colors(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<MysekaiphenomenabackgroundcolorElement>,
    >,
) -> Vec<UniversalMysekaiPhenomenaBackgroundColor> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiPhenomenaBackgroundColor> = by_id
        .values()
        .filter_map(UniversalMysekaiPhenomenaBackgroundColor::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

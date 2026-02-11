//! UniversalShopItem - Mixed: start_at is regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::shopitems::ShopitemElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for ShopitemElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

/// Flattened cost structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalShopItemCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_level: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalShopItem {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub shop_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_condition_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_box_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub costs: Option<Vec<UniversalShopItemCost>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_at: Option<UnifiedValue<i64>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalShopItem {
    pub fn from_regional(regional: &super::types::RegionalData<ShopitemElement>) -> Option<Self> {
        let id = get_first_value(regional, |s| s.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalShopItem {
            id,
            shop_id: get_first_value(regional, |s| s.shop_id),
            seq: get_first_value(regional, |s| s.seq),
            release_condition_id: get_first_value(regional, |s| s.release_condition_id),
            resource_box_id: get_first_value(regional, |s| s.resource_box_id),
            costs: get_first_value(regional, |s| {
                s.costs.as_ref().map(|costs| {
                    costs
                        .iter()
                        .map(|c| UniversalShopItemCost {
                            seq: c.seq.and_then(|v| v),
                            resource_id: c.cost.as_ref().and_then(|cc| cc.resource_id),
                            resource_type: c.cost.as_ref().and_then(|cc| {
                                cc.resource_type.as_ref().map(|t| format!("{:?}", t))
                            }),
                            resource_level: c.cost.as_ref().and_then(|cc| cc.resource_level),
                            quantity: c.cost.as_ref().and_then(|cc| cc.quantity),
                        })
                        .collect()
                })
            }),
            start_at: Some(merge_field(regional, |s| s.start_at)),
            available_regions,
        })
    }
}

pub fn merge_shop_items(
    region_data: std::collections::HashMap<ServerRegion, Vec<ShopitemElement>>,
) -> Vec<UniversalShopItem> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalShopItem> = by_id
        .values()
        .filter_map(UniversalShopItem::from_regional)
        .collect();
    result.sort_by_key(|s| s.id);
    result
}

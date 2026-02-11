//! UniversalMysekaiBlueprintMaterialCost - Merged blueprint material cost data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaiblueprintmysekaimaterialcosts::MysekaiblueprintmysekaimaterialcostElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaiblueprintmysekaimaterialcostElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiBlueprintMaterialCost {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_blueprint_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_material_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_blueprint_type: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiBlueprintMaterialCost {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaiblueprintmysekaimaterialcostElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiBlueprintMaterialCost {
            id,
            mysekai_blueprint_id: get_first_value(regional, |m| m.mysekai_blueprint_id),
            mysekai_material_id: get_first_value(regional, |m| m.mysekai_material_id),
            seq: get_first_value(regional, |m| m.seq),
            quantity: get_first_value(regional, |m| m.quantity),
            mysekai_blueprint_type: get_first_value(regional, |m| {
                m.mysekai_blueprint_type
                    .as_ref()
                    .and_then(|t| t.as_ref().map(|v| format!("{:?}", v)))
            }),
            available_regions,
        })
    }
}

pub fn merge_mysekai_blueprint_material_costs(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<MysekaiblueprintmysekaimaterialcostElement>,
    >,
) -> Vec<UniversalMysekaiBlueprintMaterialCost> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiBlueprintMaterialCost> = by_id
        .values()
        .filter_map(UniversalMysekaiBlueprintMaterialCost::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

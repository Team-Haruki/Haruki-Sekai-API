use serde::{Deserialize, Serialize};

pub type Mysekaiblueprinttermmysekaimaterialcost =
    Vec<MysekaiblueprinttermmysekaimaterialcostElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiblueprinttermmysekaimaterialcostElement {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub mysekai_material_id: Option<i64>,

    pub seq: Option<i64>,

    pub quantity: Option<i64>,
}

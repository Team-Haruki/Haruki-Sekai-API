use serde::{Deserialize, Serialize};

pub type Mysekaishopcost = Vec<MysekaishopcostElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaishopcostElement {
    pub id: Option<i64>,

    pub mysekai_shop_id: Option<i64>,

    pub seq: Option<i64>,

    pub resource_type: Option<String>,

    pub resource_id: Option<i64>,

    pub quantity: Option<i64>,
}

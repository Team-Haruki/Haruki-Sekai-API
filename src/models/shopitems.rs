// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Shopitem;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Shopitem = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Shopitem = Vec<ShopitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopitemElement {
    pub id: Option<i64>,

    pub shop_id: Option<i64>,

    pub seq: Option<i64>,

    pub release_condition_id: Option<i64>,

    pub resource_box_id: Option<i64>,

    pub costs: Option<Vec<CostElement>>,

    pub start_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostElement {
    pub shop_item_id:Option< Option<i64>>,

    pub seq:Option< Option<i64>>,

    pub cost: Option<CostCost>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostCost {
    pub resource_id: Option<i64>,

    pub resource_type: Option<ResourceType>,

    pub resource_level: Option<i64>,

    pub quantity: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Coin,

    Material,
}

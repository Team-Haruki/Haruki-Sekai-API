// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Bondshonor;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Bondshonor = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Bondshonor = Vec<BondshonorElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondshonorElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub bonds_group_id: Option<i64>,

    pub game_character_unit_id1: Option<i64>,

    pub game_character_unit_id2: Option<i64>,

    pub honor_rarity: Option<HonorRarity>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,

    pub description: Option<String>,

    pub levels: Option<Vec<Level>>,

    pub configurable_unit_virtual_singer: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HonorRarity {
    Low,

    Middle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    pub id: Option<i64>,

    pub bonds_honor_id: Option<i64>,

    pub level: Option<i64>,

    pub description: Option<String>,
}

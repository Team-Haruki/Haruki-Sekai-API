// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Honor;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Honor = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Honor = Vec<HonorElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub group_id: Option<i64>,

    pub honor_rarity:Option< Option<HonorRarity>>,

    pub name: Option<String>,

    pub assetbundle_name:Option< Option<String>>,

    pub levels: Option<Vec<Level>>,

    pub honor_type_id:Option< Option<i64>>,

    pub honor_mission_type:Option< Option<String>>,

    pub start_at:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HonorRarity {
    High,

    Highest,

    Low,

    Middle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    pub honor_id:Option< Option<i64>>,

    pub level: Option<i64>,

    pub bonus: Option<i64>,

    pub description: Option<String>,

    pub assetbundle_name:Option< Option<String>>,

    pub honor_rarity:Option< Option<HonorRarity>>,

    pub start_at:Option< Option<i64>>,
}

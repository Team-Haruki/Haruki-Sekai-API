// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaiblueprint;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaiblueprint = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaiblueprint = Vec<MysekaiblueprintElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiblueprintElement {
    pub id: Option<i64>,

    pub mysekai_craft_type: Option<MysekaiCraftType>,

    pub craft_target_id: Option<i64>,

    pub is_enable_sketch: Option<bool>,

    pub is_obtained_by_convert: Option<bool>,

    pub craft_count_limit:Option< Option<i64>>,

    pub is_available_without_possession:Option< Option<bool>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiCraftType {
    #[serde(rename = "mysekai_canvas")]
    MysekaiCanvas,

    #[serde(rename = "mysekai_fixture")]
    MysekaiFixture,

    #[serde(rename = "mysekai_tool")]
    MysekaiTool,
}

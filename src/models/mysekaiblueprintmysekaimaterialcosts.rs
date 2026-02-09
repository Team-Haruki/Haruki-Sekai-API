// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaiblueprintmysekaimaterialcost;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaiblueprintmysekaimaterialcost = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaiblueprintmysekaimaterialcost = Vec<MysekaiblueprintmysekaimaterialcostElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiblueprintmysekaimaterialcostElement {
    pub id: Option<i64>,

    pub mysekai_blueprint_id: Option<i64>,

    pub mysekai_material_id: Option<i64>,

    pub seq: Option<i64>,

    pub quantity: Option<i64>,

    pub mysekai_blueprint_type:Option< Option<MysekaiBlueprintType>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiBlueprintType {
    Normal,
}

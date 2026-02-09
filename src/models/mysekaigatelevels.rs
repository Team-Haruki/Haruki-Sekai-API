// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaigatelevel;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaigatelevel = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaigatelevel = Vec<MysekaigatelevelElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigatelevelElement {
    pub id: Option<i64>,

    pub mysekai_gate_id: Option<i64>,

    pub level: Option<i64>,

    pub mysekai_gate_material_group_id: Option<i64>,

    pub mysekai_gate_character_visit_count_rate_id: Option<i64>,

    pub power_bonus_rate: Option<f64>,
}

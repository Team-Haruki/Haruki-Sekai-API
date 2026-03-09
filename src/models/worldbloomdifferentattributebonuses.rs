// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Worldbloomdifferentattributebonuse;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Worldbloomdifferentattributebonuse = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Worldbloomdifferentattributebonuse = Vec<WorldbloomdifferentattributebonuseElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldbloomdifferentattributebonuseElement {
    pub attribute_count: Option<i64>,

    pub bonus_rate: Option<f64>,
}

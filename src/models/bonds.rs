// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Bond;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Bond = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Bond = Vec<BondElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondElement {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub character_id1: Option<i64>,

    pub character_id2: Option<i64>,
}

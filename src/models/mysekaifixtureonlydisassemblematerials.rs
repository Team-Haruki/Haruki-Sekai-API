// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaifixtureonlydisassemblematerial;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaifixtureonlydisassemblematerial = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaifixtureonlydisassemblematerial = Vec<MysekaifixtureonlydisassemblematerialElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaifixtureonlydisassemblematerialElement {
    pub id: Option<i64>,

    pub mysekai_fixture_id: Option<i64>,

    pub mysekai_material_id: Option<i64>,

    pub seq: Option<i64>,

    pub quantity: Option<i64>,
}

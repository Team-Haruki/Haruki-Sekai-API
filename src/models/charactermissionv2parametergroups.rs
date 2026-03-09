// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Charactermissionv2Parametergroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Charactermissionv2Parametergroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Charactermissionv2Parametergroup = Vec<Charactermissionv2ParametergroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Charactermissionv2ParametergroupElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub requirement: Option<i64>,

    pub exp: Option<i64>,

    pub quantity: Option<i64>,
}

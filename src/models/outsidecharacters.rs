// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Outsidecharacter;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Outsidecharacter = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Outsidecharacter = Vec<OutsidecharacterElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutsidecharacterElement {
    pub id: Option<i64>,

    pub seq:Option< Option<i64>>,

    pub name: Option<String>,
}

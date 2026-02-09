// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaigatecharacterlotterie;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaigatecharacterlotterie = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaigatecharacterlotterie = Vec<MysekaigatecharacterlotterieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigatecharacterlotterieElement {
    pub id: Option<i64>,

    pub mysekai_gate_id: Option<i64>,

    pub game_character_unit_id: Option<i64>,

    pub visitable_mysekai_gate_level: Option<i64>,
}

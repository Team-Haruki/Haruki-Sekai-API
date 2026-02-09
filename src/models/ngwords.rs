// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Ngword;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Ngword = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Ngword = Vec<NgwordElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NgwordElement {
    pub id:Option< Option<i64>>,

    pub word: Option<String>,
}

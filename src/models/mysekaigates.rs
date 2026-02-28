// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaigate;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaigate = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaigate = Vec<MysekaigateElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigateElement {
    pub id: Option<i64>,

    pub unit: Option<String>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,
}

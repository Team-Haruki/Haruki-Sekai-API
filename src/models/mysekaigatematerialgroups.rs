// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaigatematerialgroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaigatematerialgroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaigatematerialgroup = Vec<MysekaigatematerialgroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigatematerialgroupElement {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub mysekai_material_id: Option<i64>,

    pub quantity: Option<i64>,
}

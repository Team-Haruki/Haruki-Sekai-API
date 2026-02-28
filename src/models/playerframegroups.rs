// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Playerframegroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Playerframegroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Playerframegroup = Vec<PlayerframegroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerframegroupElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,
}

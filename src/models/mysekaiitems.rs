// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaiitem;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaiitem = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaiitem = Vec<MysekaiitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiitemElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub mysekai_item_type: Option<String>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,

    pub description: Option<String>,

    pub icon_assetbundle_name: Option<String>,
}

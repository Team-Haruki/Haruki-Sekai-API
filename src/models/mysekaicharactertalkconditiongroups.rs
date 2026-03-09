// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaicharactertalkconditiongroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaicharactertalkconditiongroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaicharactertalkconditiongroup = Vec<MysekaicharactertalkconditiongroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaicharactertalkconditiongroupElement {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub mysekai_character_talk_condition_id: Option<i64>,
}

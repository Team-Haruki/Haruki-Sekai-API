// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Cardraritie;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Cardraritie = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Cardraritie = Vec<CardraritieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardraritieElement {
    pub card_rarity_type: Option<String>,

    pub seq: Option<i64>,

    pub max_level: Option<i64>,

    pub max_skill_level: Option<i64>,

    pub training_max_level:Option< Option<i64>>,
}

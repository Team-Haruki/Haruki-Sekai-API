// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaimaterialgamecharacterrelation;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaimaterialgamecharacterrelation = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaimaterialgamecharacterrelation = Vec<MysekaimaterialgamecharacterrelationElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaimaterialgamecharacterrelationElement {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub mysekai_material_id: Option<i64>,

    pub game_character_id: Option<i64>,
}

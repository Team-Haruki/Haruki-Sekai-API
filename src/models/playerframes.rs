// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Playerframe;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Playerframe = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Playerframe = Vec<PlayerframeElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerframeElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub player_frame_group_id: Option<i64>,

    pub description: Option<String>,

    pub game_character_id: Option<i64>,
}

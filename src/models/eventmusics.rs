// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventmusic;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventmusic = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Eventmusic = Vec<EventmusicElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventmusicElement {
    pub event_id: Option<i64>,

    pub music_id: Option<i64>,

    pub seq: Option<i64>,

    pub release_condition_id: Option<i64>,
}

// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventcard;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventcard = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Eventcard = Vec<EventcardElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventcardElement {
    pub id: Option<i64>,

    pub card_id: Option<i64>,

    pub event_id: Option<i64>,

    pub bonus_rate: Option<f64>,

    pub leader_bonus_rate: Option<f64>,

    pub is_display_card_story: Option<bool>,
}

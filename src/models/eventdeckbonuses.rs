// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventdeckbonuse;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventdeckbonuse = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Eventdeckbonuse = Vec<EventdeckbonuseElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventdeckbonuseElement {
    pub id: Option<i64>,

    pub event_id: Option<i64>,

    pub game_character_unit_id: Option<i64>,

    pub card_attr: Option<CardAttr>,

    pub bonus_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardAttr {
    Cool,

    Cute,

    Happy,

    Mysterious,

    Pure,
}

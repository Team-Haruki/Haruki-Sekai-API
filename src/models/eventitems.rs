// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventitem;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventitem = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Eventitem = Vec<EventitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventitemElement {
    pub id: Option<i64>,

    pub event_id: Option<i64>,

    pub name: Option<String>,

    pub flavor_text: Option<String>,

    pub assetbundle_name: Option<String>,

    pub game_character_id: Option<i64>,
}

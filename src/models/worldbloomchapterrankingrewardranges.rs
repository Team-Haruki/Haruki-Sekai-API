// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Worldbloomchapterrankingrewardrange;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Worldbloomchapterrankingrewardrange = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Worldbloomchapterrankingrewardrange = Vec<WorldbloomchapterrankingrewardrangeElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldbloomchapterrankingrewardrangeElement {
    pub id: Option<i64>,

    pub event_id: Option<i64>,

    pub game_character_id: Option<i64>,

    pub from_rank: Option<i64>,

    pub to_rank: Option<i64>,

    pub is_to_rank_border: Option<bool>,

    pub resource_box_id: Option<i64>,
}

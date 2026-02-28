// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Cardmysekaicanvasbonuse;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Cardmysekaicanvasbonuse = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Cardmysekaicanvasbonuse = Vec<CardmysekaicanvasbonuseElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardmysekaicanvasbonuseElement {
    pub id: Option<i64>,

    pub card_rarity_type: Option<String>,

    pub power1_bonus_fixed: Option<i64>,

    pub power2_bonus_fixed: Option<i64>,

    pub power3_bonus_fixed: Option<i64>,
}

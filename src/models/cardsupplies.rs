// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Cardsupplie;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Cardsupplie = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Cardsupplie = Vec<CardsupplieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardsupplieElement {
    pub id: Option<i64>,

    pub card_supply_type: Option<String>,

    pub assetbundle_name: Option<String>,
}

// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventraritybonusrate;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventraritybonusrate = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Eventraritybonusrate = Vec<EventraritybonusrateElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventraritybonusrateElement {
    pub id: Option<i64>,

    pub card_rarity_type: Option<CardRarityType>,

    pub master_rank: Option<i64>,

    pub bonus_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardRarityType {
    #[serde(rename = "rarity_1")]
    Rarity1,

    #[serde(rename = "rarity_2")]
    Rarity2,

    #[serde(rename = "rarity_3")]
    Rarity3,

    #[serde(rename = "rarity_4")]
    Rarity4,

    #[serde(rename = "rarity_birthday")]
    RarityBirthday,
}

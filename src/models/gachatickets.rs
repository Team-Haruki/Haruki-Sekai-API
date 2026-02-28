// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Gachaticket;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Gachaticket = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Gachaticket = Vec<GachaticketElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaticketElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub gacha_display_type: Option<GachaDisplayType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GachaDisplayType {
    Always,

    Having,
}

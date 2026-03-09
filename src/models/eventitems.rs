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

    pub name: Option<Name>,

    pub flavor_text: Option<FlavorText>,

    pub assetbundle_name: Option<String>,

    pub game_character_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FlavorText {
    #[serde(rename = "イベント交換所でアイテムと交換できます。")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Name {
    #[serde(rename = "イベントバッジ")]
    Empty,

    #[serde(rename = "活动徽章")]
    Fluffy,

    #[serde(rename = "ワールドバッジ")]
    Name,

    #[serde(rename = "章节1徽章")]
    Name1,

    #[serde(rename = "章节2徽章")]
    Name2,

    #[serde(rename = "章节3徽章")]
    Name3,

    #[serde(rename = "章节4徽章")]
    Name4,

    #[serde(rename = "章节5徽章")]
    Name5,

    #[serde(rename = "章节6徽章")]
    Name6,

    #[serde(rename = "フィナーレバッジ")]
    Purple,

    #[serde(rename = "世界徽章")]
    Tentacled,

    #[serde(rename = "チャプター1バッジ")]
    The1,

    #[serde(rename = "チャプター2バッジ")]
    The2,

    #[serde(rename = "チャプター3バッジ")]
    The3,

    #[serde(rename = "チャプター4バッジ")]
    The4,

    #[serde(rename = "チャプター5バッジ")]
    The5,

    #[serde(rename = "チャプター6バッジ")]
    The6,
}

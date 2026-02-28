// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Honorgroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Honorgroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Honorgroup = Vec<HonorgroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorgroupElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,

    pub honor_type: Option<HonorType>,

    pub background_assetbundle_name:Option<String>,

    pub frame_name:Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HonorType {
    Achievement,

    Birthday,

    Character,

    Event,

    Limitevent,

    #[serde(rename = "rank_match")]
    RankMatch,

    #[serde(rename = "sekai_echo")]
    SekaiEcho,
}

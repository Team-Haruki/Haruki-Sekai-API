// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Worldbloom;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Worldbloom = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Worldbloom = Vec<WorldbloomElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldbloomElement {
    pub id: Option<i64>,

    pub event_id: Option<i64>,

    pub game_character_id:Option<i64>,

    pub world_bloom_chapter_type:Option<WorldBloomChapterType>,

    pub chapter_no: Option<i64>,

    pub chapter_start_at: Option<i64>,

    pub aggregate_at: Option<i64>,

    pub chapter_end_at: Option<i64>,

    pub is_supplemental: Option<bool>,

    #[serde(rename = "costume2dId")]
    pub costume2_d_id:Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldBloomChapterType {
    Finale,

    #[serde(rename = "game_character")]
    GameCharacter,
}

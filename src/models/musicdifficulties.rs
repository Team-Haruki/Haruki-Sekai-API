// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Musicdifficultie;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Musicdifficultie = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Musicdifficultie = Vec<MusicdifficultieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicdifficultieElement {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_difficulty: Option<MusicDifficulty>,

    pub play_level: Option<i64>,

    pub total_note_count: Option<i64>,

    pub release_condition_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicDifficulty {
    Append,

    Easy,

    Expert,

    Hard,

    Master,

    Normal,
}

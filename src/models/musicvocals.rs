// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Musicvocal;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Musicvocal = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Musicvocal = Vec<MusicvocalElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicvocalElement {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_vocal_type: Option<MusicVocalType>,

    pub seq: Option<i64>,

    pub release_condition_id: Option<i64>,

    pub caption: Option<String>,

    pub characters: Option<Vec<Character>>,

    pub assetbundle_name: Option<String>,

    pub archive_published_at: Option<Option<i64>>,

    pub special_season_id: Option<Option<i64>>,

    pub archive_display_type: Option<Option<ArchiveDisplayType>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDisplayType {
    None,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_vocal_id: Option<i64>,

    pub character_type: Option<CharacterType>,

    pub character_id: Option<i64>,

    pub seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterType {
    #[serde(rename = "game_character")]
    GameCharacter,

    #[serde(rename = "outside_character")]
    OutsideCharacter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicVocalType {
    #[serde(rename = "another_vocal")]
    AnotherVocal,

    #[serde(rename = "april_fool_2022")]
    AprilFool2022,

    Instrumental,

    #[serde(rename = "original_song")]
    OriginalSong,

    Sekai,

    #[serde(rename = "streaming_live")]
    StreamingLive,

    #[serde(rename = "virtual_singer")]
    VirtualSinger,
}

// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaimusicrecordcategorie;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaimusicrecordcategorie = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaimusicrecordcategorie = Vec<MysekaimusicrecordcategorieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaimusicrecordcategorieElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub seq: Option<i64>,

    pub mysekai_music_track_type: Option<MysekaiMusicTrackType>,

    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiMusicTrackType {
    Music,

    #[serde(rename = "music_sound_track")]
    MusicSoundTrack,
}

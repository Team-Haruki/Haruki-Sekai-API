// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaimusicrecord;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaimusicrecord = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaimusicrecord = Vec<MysekaimusicrecordElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaimusicrecordElement {
    pub id: Option<i64>,

    pub mysekai_music_track_type: Option<MysekaiMusicTrackType>,

    pub external_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiMusicTrackType {
    Music,

    #[serde(rename = "music_sound_track")]
    MusicSoundTrack,
}

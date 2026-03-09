// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Musictag;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Musictag = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Musictag = Vec<MusictagElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusictagElement {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_tag: Option<MusicTag>,

    pub seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicTag {
    All,

    Idol,

    #[serde(rename = "light_music_club")]
    LightMusicClub,

    Other,

    #[serde(rename = "school_refusal")]
    SchoolRefusal,

    Street,

    #[serde(rename = "theme_park")]
    ThemePark,

    Vocaloid,
}

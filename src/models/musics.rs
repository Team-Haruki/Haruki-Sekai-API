// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Music;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Music = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Music = Vec<MusicElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub release_condition_id: Option<i64>,

    pub categories: Option<Vec<CategoryElement>>,

    pub title: Option<String>,

    pub pronunciation: Option<String>,

    pub creator_artist_id: Option<i64>,

    pub lyricist: Option<String>,

    pub composer: Option<String>,

    pub arranger: Option<String>,

    pub dancer_count: Option<i64>,

    pub self_dancer_position: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub live_talk_background_assetbundle_name:Option<LiveTalkBackgroundAssetbundleName>,

    pub published_at: Option<i64>,

    pub released_at: Option<i64>,

    pub live_stage_id:Option<i64>,

    pub filler_sec: Option<f64>,

    pub is_newly_written_music: Option<bool>,

    pub is_full_length: Option<bool>,

    pub music_collaboration_id:Option<i64>,

    pub infos:Option<Vec<Info>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CategoryElement {
    CategoryClass(CategoryClass),

    Enum(CategoryEnum),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryClass {
    pub music_category_name: Option<CategoryEnum>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryEnum {
    Image,

    Mv,

    #[serde(rename = "mv_2d")]
    Mv2D,

    Original,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub title: Option<String>,

    pub creator: Option<String>,

    pub lyricist: Option<String>,

    pub composer: Option<String>,

    pub arranger: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveTalkBackgroundAssetbundleName {
    #[serde(rename = "bg_livetalk_default_002")]
    BgLivetalkDefault002,
}

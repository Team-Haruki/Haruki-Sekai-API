// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Stamp;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Stamp = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Stamp = Vec<StampElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StampElement {
    pub id: Option<i64>,

    pub stamp_type: Option<StampType>,

    pub seq: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub balloon_assetbundle_name: Option<BalloonAssetbundleName>,

    pub character_id1:Option<i64>,

    pub game_character_unit_id:Option<i64>,

    pub archive_published_at: Option<i64>,

    pub description:Option<String>,

    pub archive_display_type:Option<ArchiveDisplayType>,

    pub character_id2:Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDisplayType {
    Hide,

    None,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BalloonAssetbundleName {
    #[serde(rename = "balloon_stamp_01")]
    BalloonStamp01,

    #[serde(rename = "balloon_stamp_02")]
    BalloonStamp02,

    #[serde(rename = "balloon_stamp_03")]
    BalloonStamp03,

    #[serde(rename = "balloon_stamp_04")]
    BalloonStamp04,

    #[serde(rename = "balloon_stamp_05")]
    BalloonStamp05,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StampType {
    #[serde(rename = "cheerful_carnival_message")]
    CheerfulCarnivalMessage,

    Illustration,

    #[serde(rename = "non_character_illustration")]
    NonCharacterIllustration,

    Text,
}

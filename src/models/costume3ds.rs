// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Costume3D;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Costume3D = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Costume3D = Vec<Costume3DElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Costume3DElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    #[serde(rename = "costume3dGroupId")]
    pub costume3_d_group_id: Option<i64>,

    #[serde(rename = "costume3dType")]
    pub costume3_d_type: Option<Costume3DType>,

    pub name: Option<String>,

    pub part_type: Option<PartType>,

    pub color_id: Option<i64>,

    pub color_name: Option<ColorName>,

    pub character_id: Option<i64>,

    #[serde(rename = "costume3dRarity")]
    pub costume3_d_rarity: Option<Costume3DRarity>,

    pub how_to_obtain:Option< Option<String>>,

    pub assetbundle_name: Option<String>,

    pub designer: Option<String>,

    pub archive_display_type:Option< Option<ArchiveDisplayType>>,

    pub archive_published_at: Option<i64>,

    pub published_at:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDisplayType {
    None,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColorName {
    #[serde(rename = "オリジナル")]
    ColorName,

    #[serde(rename = "其他颜色1")]
    ColorName1,

    #[serde(rename = "其他颜色2")]
    ColorName2,

    #[serde(rename = "其他颜色3")]
    ColorName3,

    #[serde(rename = "ノーマル")]
    Empty,

    #[serde(rename = "原版颜色")]
    Fluffy,

    #[serde(rename = "普通")]
    Purple,

    #[serde(rename = "限定颜色")]
    Tentacled,

    #[serde(rename = "アナザー1")]
    The1,

    #[serde(rename = "アナザー2")]
    The2,

    #[serde(rename = "アナザー3")]
    The3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Costume3DRarity {
    Normal,

    Rare,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Costume3DType {
    Default,

    Distribution,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartType {
    Body,

    Hair,

    Head,
}

// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Character2D;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Character2D = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Character2D = Vec<Character2DElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character2DElement {
    pub id: Option<i64>,

    pub character_type: Option<CharacterType>,

    pub is_next_grade: Option<bool>,

    pub character_id: Option<i64>,

    pub unit: Option<Unit>,

    pub is_enabled_flip_display: Option<bool>,

    pub asset_name: Option<String>,

    pub character_icon_assetbundle_name: Option<CharacterIconAssetbundleName>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterIconAssetbundleName {
    #[serde(rename = "aprilfool_2025")]
    Aprilfool2025,

    #[serde(rename = "e_collabo")]
    ECollabo,

    #[serde(rename = "egg_collabo")]
    EggCollabo,

    #[serde(rename = "s_collabo")]
    SCollabo,

    Sanrio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterType {
    #[serde(rename = "game_character")]
    GameCharacter,

    Mob,

    #[serde(rename = "sub_game_character")]
    SubGameCharacter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Idol,

    #[serde(rename = "light_sound")]
    LightSound,

    None,

    Piapro,

    #[serde(rename = "school_refusal")]
    SchoolRefusal,

    Street,

    #[serde(rename = "theme_park")]
    ThemePark,
}

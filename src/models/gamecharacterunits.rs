// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Gamecharacterunit;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Gamecharacterunit = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Gamecharacterunit = Vec<GamecharacterunitElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GamecharacterunitElement {
    pub id: Option<i64>,

    pub game_character_id: Option<i64>,

    pub unit: Option<Unit>,

    pub color_code: Option<String>,

    pub skin_color_code: Option<SkinColorCode>,

    pub skin_shadow_color_code1: Option<SkinShadowColorCode1>,

    pub skin_shadow_color_code2: Option<SkinShadowColorCode2>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SkinColorCode {
    #[serde(rename = "#feefe0")]
    Feefe0,

    #[serde(rename = "#fef6ec")]
    Fef6Ec,

    #[serde(rename = "#fff5e8")]
    Fff5E8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SkinShadowColorCode1 {
    #[serde(rename = "#e4c5cc")]
    E4C5Cc,

    #[serde(rename = "#eca9aa")]
    Eca9Aa,

    #[serde(rename = "#efafbb")]
    Efafbb,

    #[serde(rename = "#f4b6bc")]
    F4B6Bc,

    #[serde(rename = "#f4b6cd")]
    F4B6Cd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SkinShadowColorCode2 {
    #[serde(rename = "#cc98a3")]
    Cc98A3,

    #[serde(rename = "#da7071")]
    Da7071,

    #[serde(rename = "#e07889")]
    E07889,

    #[serde(rename = "#e9828b")]
    E9828B,

    #[serde(rename = "#e982a5")]
    E982A5,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Idol,

    #[serde(rename = "light_sound")]
    LightSound,

    Piapro,

    #[serde(rename = "school_refusal")]
    SchoolRefusal,

    Street,

    #[serde(rename = "theme_park")]
    ThemePark,
}

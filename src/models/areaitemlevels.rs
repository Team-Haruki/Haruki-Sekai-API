// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Areaitemlevel;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Areaitemlevel = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Areaitemlevel = Vec<AreaitemlevelElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaitemlevelElement {
    pub area_item_id: Option<i64>,

    pub level: Option<i64>,

    pub target_unit: Option<TargetUnit>,

    pub target_card_attr: Option<TargetCardAttr>,

    pub target_game_character_id: Option<i64>,

    pub power1_bonus_rate: Option<f64>,

    pub power1_all_match_bonus_rate: Option<f64>,

    pub power2_bonus_rate: Option<f64>,

    pub power2_all_match_bonus_rate: Option<f64>,

    pub power3_bonus_rate: Option<f64>,

    pub power3_all_match_bonus_rate: Option<f64>,

    pub sentence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetCardAttr {
    Any,

    Cool,

    Cute,

    Happy,

    Mysterious,

    Pure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetUnit {
    Any,

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

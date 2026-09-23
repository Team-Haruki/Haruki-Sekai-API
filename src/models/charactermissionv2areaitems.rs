// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Charactermissionv2Areaitem;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Charactermissionv2Areaitem = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Charactermissionv2Areaitem = Vec<Charactermissionv2AreaitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Charactermissionv2AreaitemElement {
    pub id: Option<i64>,

    pub character_mission_type: Option<CharacterMissionType>,

    pub area_item_id: Option<i64>,

    pub character_id: Option<i64>,

    pub unit: Option<Unit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterMissionType {
    #[serde(rename = "area_item_level_up_character")]
    AreaItemLevelUpCharacter,

    #[serde(rename = "area_item_level_up_reality_world")]
    AreaItemLevelUpRealityWorld,

    #[serde(rename = "area_item_level_up_unit")]
    AreaItemLevelUpUnit,
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

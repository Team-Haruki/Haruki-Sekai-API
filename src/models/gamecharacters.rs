// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Gamecharacter;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Gamecharacter = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Gamecharacter = Vec<GamecharacterElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GamecharacterElement {
    pub id: Option<i64>,

    pub seq:Option<i64>,

    pub resource_id: Option<i64>,

    pub first_name:Option<String>,

    pub given_name: Option<String>,

    pub first_name_ruby:Option<String>,

    pub given_name_ruby:Option<String>,

    pub first_name_english:Option<String>,

    pub given_name_english: Option<String>,

    pub gender: Option<Gender>,

    pub height: Option<f64>,

    #[serde(rename = "live2dHeightAdjustment")]
    pub live2_d_height_adjustment: Option<f64>,

    pub figure: Option<Figure>,

    pub breast_size: Option<BreastSize>,

    pub model_name:Option<String>,

    pub unit: Option<Unit>,

    pub support_unit_type: Option<SupportUnitType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreastSize {
    L,

    M,

    None,

    S,

    Ss,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Figure {
    Boys,

    Ladies,

    Mens,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gender {
    Female,

    Male,

    Secret,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportUnitType {
    Full,

    None,

    Unit,
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

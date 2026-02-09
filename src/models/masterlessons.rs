// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Masterlesson;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Masterlesson = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Masterlesson = Vec<MasterlessonElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterlessonElement {
    pub card_rarity_type: Option<CardRarityType>,

    pub master_rank: Option<i64>,

    pub power1_bonus_fixed: Option<i64>,

    pub power2_bonus_fixed: Option<i64>,

    pub power3_bonus_fixed: Option<i64>,

    pub character_rank_exp:Option< Option<i64>>,

    pub costs: Option<Vec<Cost>>,

    pub rewards:Option< Option<Vec<Option<serde_json::Value>>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardRarityType {
    #[serde(rename = "rarity_1")]
    Rarity1,

    #[serde(rename = "rarity_2")]
    Rarity2,

    #[serde(rename = "rarity_3")]
    Rarity3,

    #[serde(rename = "rarity_4")]
    Rarity4,

    #[serde(rename = "rarity_birthday")]
    RarityBirthday,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub id: Option<i64>,

    pub card_rarity_type: Option<CardRarityType>,

    pub master_rank: Option<i64>,

    pub seq:Option< Option<i64>>,

    pub resource_type:Option< Option<ResourceType>>,

    pub resource_id: Option<i64>,

    pub quantity: Option<i64>,

    pub character_id:Option< Option<i64>>,

    pub unit:Option< Option<Unit>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Material,
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

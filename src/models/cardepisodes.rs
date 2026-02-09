// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Cardepisode;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Cardepisode = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Cardepisode = Vec<CardepisodeElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardepisodeElement {
    pub id: Option<i64>,

    pub seq:Option< Option<i64>>,

    pub card_id: Option<i64>,

    pub title: Option<Title>,

    pub scenario_id: Option<String>,

    pub assetbundle_name:Option< Option<String>>,

    pub release_condition_id: Option<i64>,

    pub power1_bonus_fixed: Option<i64>,

    pub power2_bonus_fixed: Option<i64>,

    pub power3_bonus_fixed: Option<i64>,

    pub reward_resource_box_ids:Option< Option<Vec<i64>>>,

    pub costs: Option<Vec<Cost>>,

    pub card_episode_part_type: Option<CardEpisodePartType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardEpisodePartType {
    #[serde(rename = "first_part")]
    FirstPart,

    #[serde(rename = "second_part")]
    SecondPart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub resource_id: Option<i64>,

    pub resource_type: Option<ResourceType>,

    pub quantity: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Material,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Title {
    #[serde(rename = "サイドストーリー（前編）")]
    Empty,

    #[serde(rename = "卡牌剧情（下篇）")]
    Fluffy,

    #[serde(rename = "卡牌剧情（上篇）")]
    Purple,

    #[serde(rename = "サイドストーリー（後編）")]
    Title,
}

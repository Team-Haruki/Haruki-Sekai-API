// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Worldbloomsupportdeckbonuse;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Worldbloomsupportdeckbonuse = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Worldbloomsupportdeckbonuse = Vec<WorldbloomsupportdeckbonuseElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldbloomsupportdeckbonuseElement {
    pub card_rarity_type: Option<String>,

    pub world_bloom_support_deck_character_bonuses: Option<Vec<WorldBloomSupportDeckCharacterBonus>>,

    pub world_bloom_support_deck_master_rank_bonuses: Option<Vec<WorldBloomSupportDeckMasterRankBonus>>,

    pub world_bloom_support_deck_skill_level_bonuses: Option<Vec<WorldBloomSupportDeckSkillLevelBonus>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldBloomSupportDeckCharacterBonus {
    pub id: Option<i64>,

    pub world_bloom_support_deck_character_type: Option<WorldBloomSupportDeckCharacterType>,

    pub bonus_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldBloomSupportDeckCharacterType {
    Others,

    Specific,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldBloomSupportDeckMasterRankBonus {
    pub id: Option<i64>,

    pub master_rank: Option<i64>,

    pub bonus_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldBloomSupportDeckSkillLevelBonus {
    pub id: Option<i64>,

    pub skill_level: Option<i64>,

    pub bonus_rate: Option<f64>,
}

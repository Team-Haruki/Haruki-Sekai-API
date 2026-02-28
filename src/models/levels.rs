// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Level;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Level = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Level = Vec<LevelElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelElement {
    pub id:Option<i64>,

    pub level_type: Option<LevelType>,

    pub level: Option<i64>,

    pub total_exp: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelType {
    Bonds,

    Card,

    #[serde(rename = "card_skill_1")]
    CardSkill1,

    #[serde(rename = "card_skill_2")]
    CardSkill2,

    #[serde(rename = "card_skill_3")]
    CardSkill3,

    #[serde(rename = "card_skill_4")]
    CardSkill4,

    #[serde(rename = "card_skill_birthday")]
    CardSkillBirthday,

    Character,

    Unit,

    User,
}

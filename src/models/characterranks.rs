// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Characterrank;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Characterrank = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Characterrank = Vec<CharacterrankElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterrankElement {
    pub id: Option<i64>,

    pub character_id: Option<i64>,

    pub character_rank: Option<i64>,

    pub power1_bonus_rate: Option<f64>,

    pub power2_bonus_rate: Option<f64>,

    pub power3_bonus_rate: Option<f64>,

    pub reward_resource_box_ids: Option<Vec<i64>>,

    pub character_rank_achieve_resources: Option<Vec<CharacterRankAchieveResource>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterRankAchieveResource {
    pub release_condition_id:Option< Option<i64>>,

    pub character_id:Option< Option<i64>>,

    pub character_rank:Option< Option<i64>>,

    pub resources: Option<Vec<Option<serde_json::Value>>>,
}

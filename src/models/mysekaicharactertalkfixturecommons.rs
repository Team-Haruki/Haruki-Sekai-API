// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaicharactertalkfixturecommon;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaicharactertalkfixturecommon = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaicharactertalkfixturecommon = Vec<MysekaicharactertalkfixturecommonElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaicharactertalkfixturecommonElement {
    pub id: Option<i64>,

    pub game_character_unit_id: Option<i64>,

    pub mysekai_character_talk_fixture_common_type: Option<MysekaiCharacterTalkFixtureCommonType>,

    pub mysekai_character_talk_fixture_common_mysekai_fixture_group_id: Option<i64>,

    pub mysekai_character_talk_fixture_common_tweet_group_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiCharacterTalkFixtureCommonType {
    Normal,

    Positive,
}

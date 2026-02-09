// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaicharactertalkcondition;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaicharactertalkcondition = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaicharactertalkcondition = Vec<MysekaicharactertalkconditionElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaicharactertalkconditionElement {
    pub id: Option<i64>,

    pub mysekai_character_talk_condition_type: Option<MysekaiCharacterTalkConditionType>,

    pub mysekai_character_talk_condition_type_value: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiCharacterTalkConditionType {
    #[serde(rename = "mysekai_character_visit_count")]
    MysekaiCharacterVisitCount,

    #[serde(rename = "mysekai_fixture_id")]
    MysekaiFixtureId,

    #[serde(rename = "mysekai_phenomena_id")]
    MysekaiPhenomenaId,

    #[serde(rename = "read_event_story_episode_id")]
    ReadEventStoryEpisodeId,
}

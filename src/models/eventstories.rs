// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventstorie;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventstorie = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Eventstorie = Vec<EventstorieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventstorieElement {
    pub id: Option<i64>,

    pub event_id: Option<i64>,

    pub outline: Option<String>,

    pub banner_game_character_unit_id:Option<i64>,

    pub assetbundle_name: Option<String>,

    pub event_story_episodes: Option<Vec<EventStoryEpisode>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventStoryEpisode {
    pub id: Option<i64>,

    pub event_story_id: Option<i64>,

    pub episode_no: Option<i64>,

    pub title: Option<String>,

    pub assetbundle_name: Option<String>,

    pub scenario_id: Option<String>,

    pub release_condition_id: Option<i64>,

    pub episode_rewards: Option<Vec<EpisodeReward>>,

    pub game_character_id:Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeReward {
    pub story_type:Option<StoryType>,

    pub resource_box_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryType {
    #[serde(rename = "event_story")]
    EventStory,
}

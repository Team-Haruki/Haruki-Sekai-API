// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventstoryunit;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventstoryunit = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Eventstoryunit = Vec<EventstoryunitElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventstoryunitElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub event_story_id: Option<i64>,

    pub unit: Option<Unit>,

    pub event_story_unit_relation: Option<EventStoryUnitRelation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStoryUnitRelation {
    Main,

    Sub,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Idol,

    #[serde(rename = "light_sound")]
    LightSound,

    #[serde(rename = "school_refusal")]
    SchoolRefusal,

    Street,

    #[serde(rename = "theme_park")]
    ThemePark,
}

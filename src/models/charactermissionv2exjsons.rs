// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Charactermissionv2Exjson;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Charactermissionv2Exjson = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Charactermissionv2Exjson = Vec<Charactermissionv2ExjsonElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Charactermissionv2ExjsonElement {
    pub id: Option<i64>,

    pub character_mission_ex_type: Option<CharacterMissionExType>,

    pub character_mission_type: Option<CharacterMissionType>,

    pub resource_type: Option<ResourceType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterMissionExType {
    #[serde(rename = "play_live_ex")]
    PlayLiveEx,

    #[serde(rename = "waiting_room_ex")]
    WaitingRoomEx,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterMissionType {
    #[serde(rename = "play_live")]
    PlayLive,

    #[serde(rename = "waiting_room")]
    WaitingRoom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Material,
}

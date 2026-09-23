// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Charactermissionv2;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Charactermissionv2 = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Charactermissionv2 = Vec<Charactermissionv2Element>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Charactermissionv2Element {
    pub id: Option<i64>,

    pub character_mission_type: Option<CharacterMissionType>,

    pub character_id: Option<i64>,

    pub parameter_group_id: Option<i64>,

    pub sentence: Option<String>,

    pub progress_sentence: Option<String>,

    pub is_achievement_mission: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterMissionType {
    #[serde(rename = "area_item_level_up_character")]
    AreaItemLevelUpCharacter,

    #[serde(rename = "area_item_level_up_reality_world")]
    AreaItemLevelUpRealityWorld,

    #[serde(rename = "area_item_level_up_unit")]
    AreaItemLevelUpUnit,

    #[serde(rename = "collect_another_vocal")]
    CollectAnotherVocal,

    #[serde(rename = "collect_character_archive_voice")]
    CollectCharacterArchiveVoice,

    #[serde(rename = "collect_costume_3d")]
    CollectCostume3d,

    #[serde(rename = "collect_member")]
    CollectMember,

    #[serde(rename = "collect_mysekai_canvas")]
    CollectMysekaiCanvas,

    #[serde(rename = "collect_mysekai_fixture")]
    CollectMysekaiFixture,

    #[serde(rename = "collect_stamp")]
    CollectStamp,

    #[serde(rename = "master_rank_up_rare")]
    MasterRankUpRare,

    #[serde(rename = "master_rank_up_standard")]
    MasterRankUpStandard,

    #[serde(rename = "play_live")]
    PlayLive,

    #[serde(rename = "play_live_ex")]
    PlayLiveEx,

    #[serde(rename = "read_area_talk")]
    ReadAreaTalk,

    #[serde(rename = "read_card_episode_first")]
    ReadCardEpisodeFirst,

    #[serde(rename = "read_card_episode_second")]
    ReadCardEpisodeSecond,

    #[serde(rename = "read_mysekai_fixture_unique_character_talk")]
    ReadMysekaiFixtureUniqueCharacterTalk,

    #[serde(rename = "skill_level_up_rare")]
    SkillLevelUpRare,

    #[serde(rename = "skill_level_up_standard")]
    SkillLevelUpStandard,

    #[serde(rename = "waiting_room")]
    WaitingRoom,

    #[serde(rename = "waiting_room_ex")]
    WaitingRoomEx,
}

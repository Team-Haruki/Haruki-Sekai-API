// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Event;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Event = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Event = Vec<EventElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventElement {
    pub id: Option<i64>,

    pub event_type: Option<EventType>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub bgm_assetbundle_name: Option<String>,

    pub event_only_component_display_start_at: Option<i64>,

    pub start_at: Option<i64>,

    pub aggregate_at: Option<i64>,

    pub ranking_announce_at: Option<i64>,

    pub distribution_start_at: Option<i64>,

    pub event_only_component_display_end_at: Option<i64>,

    pub closed_at: Option<i64>,

    pub distribution_end_at: Option<i64>,

    pub virtual_live_id:Option<i64>,

    pub unit: Option<Unit>,

    pub is_count_leader_character_play:Option<bool>,

    pub event_ranking_reward_ranges: Option<Vec<EventRankingRewardRange>>,

    pub event_point_assetbundle_name:Option<EventPointAssetbundleName>,

    pub standby_screen_display_start_at:Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPointAssetbundleName {
    #[serde(rename = "icon_point")]
    IconPoint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRankingRewardRange {
    pub id:Option<i64>,

    pub event_id:Option<i64>,

    pub from_rank: Option<i64>,

    pub to_rank: Option<i64>,

    pub is_to_rank_border:Option<bool>,

    pub event_ranking_rewards: Option<Vec<EventRankingReward>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRankingReward {
    pub id:Option<i64>,

    pub event_ranking_reward_range_id:Option<i64>,

    pub seq:Option<i64>,

    pub resource_box_id: Option<i64>,

    pub reward_condition_type:Option<RewardConditionType>,

    pub condition_value:Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewardConditionType {
    #[serde(rename = "leader_character_play_count")]
    LeaderCharacterPlayCount,

    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    #[serde(rename = "cheerful_carnival")]
    CheerfulCarnival,

    Marathon,

    #[serde(rename = "world_bloom")]
    WorldBloom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Idol,

    #[serde(rename = "light_sound")]
    LightSound,

    None,

    #[serde(rename = "school_refusal")]
    SchoolRefusal,

    Street,

    #[serde(rename = "theme_park")]
    ThemePark,
}

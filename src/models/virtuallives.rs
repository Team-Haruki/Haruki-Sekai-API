// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Virtuallive;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Virtuallive = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Virtuallive = Vec<VirtualliveElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualliveElement {
    pub id: Option<i64>,

    pub virtual_live_type: Option<VirtualLiveType>,

    pub virtual_live_platform: Option<VirtualLivePlatform>,

    pub seq: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub screen_mv_music_vocal_id: Option<i64>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,

    pub ranking_announce_at: Option<i64>,

    pub virtual_live_setlists: Option<Vec<VirtualLiveSetlist>>,

    pub virtual_live_beginner_schedules: Option<Vec<VirtualLiveBeginnerSchedule>>,

    pub virtual_live_schedules: Option<Vec<VirtualLiveSchedule>>,

    pub virtual_live_characters: Option<Vec<VirtualLiveCharacter>>,

    pub virtual_live_rewards: Option<Vec<VirtualLiveReward>>,

    pub virtual_live_cheer_point_rewards: Option<Vec<Option<serde_json::Value>>>,

    pub virtual_live_waiting_room: Option<VirtualLiveWaitingRoom>,

    pub virtual_items: Option<Vec<VirtualItem>>,

    pub virtual_live_appeals: Option<Vec<VirtualLiveAppeal>>,

    pub virtual_live_background_musics: Option<Vec<VirtualLiveBackgroundMusic>>,

    pub virtual_live_information: Option<VirtualLiveInformation>,

    pub archive_release_condition_id: Option<i64>,

    pub sub_game_character_penlight_color_group_id: Option<i64>,

    pub virtual_live_group_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualItem {
    pub id: Option<i64>,

    pub virtual_item_category: Option<VirtualItemCategory>,

    pub seq: Option<i64>,

    pub priority: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub cost_virtual_coin: Option<i64>,

    pub cost_jewel: Option<i64>,

    pub effect_assetbundle_name: Option<String>,

    pub effect_expression_type: Option<EffectExpressionType>,

    pub unit: Option<Unit>,

    pub game_character_unit_id: Option<i64>,

    pub virtual_item_label_type: Option<VirtualItemLabelType>,

    pub sub_game_character_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectExpressionType {
    #[serde(rename = "throw_effect")]
    ThrowEffect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Idol,

    #[serde(rename = "light_sound")]
    LightSound,

    Piapro,

    #[serde(rename = "school_refusal")]
    SchoolRefusal,

    Street,

    #[serde(rename = "theme_park")]
    ThemePark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualItemCategory {
    Normal,

    Spread,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualItemLabelType {
    Special,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveAppeal {
    pub id: Option<i64>,

    pub virtual_live_id: Option<i64>,

    pub virtual_live_stage_status: Option<VirtualLiveStageStatus>,

    pub appeal_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualLiveStageStatus {
    Live,

    Open,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveBackgroundMusic {
    pub id: Option<i64>,

    pub virtual_live_id: Option<i64>,

    pub background_music_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveBeginnerSchedule {
    pub id: Option<i64>,

    pub virtual_live_id: Option<i64>,

    pub day_of_week: Option<DayOfWeek>,

    pub start_time: Option<String>,

    pub end_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayOfWeek {
    Friday,

    Monday,

    Saturday,

    Sunday,

    Thursday,

    Tuesday,

    Wednesday,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveCharacter {
    pub id: Option<i64>,

    pub virtual_live_id: Option<i64>,

    pub game_character_unit_id: Option<i64>,

    pub seq: Option<i64>,

    pub virtual_live_performance_type: Option<VirtualLivePerformanceType>,

    #[serde(rename = "subGameCharacter2dId")]
    pub sub_game_character2_d_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualLivePerformanceType {
    Both,

    #[serde(rename = "main_only")]
    MainOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveInformation {
    pub virtual_live_id: Option<i64>,

    pub summary: Option<Summary>,

    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Summary {
    #[serde(rename = "コネクトライブ")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualLivePlatform {
    V1,

    V2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveReward {
    pub id: Option<i64>,

    pub virtual_live_type: Option<VirtualLiveType>,

    pub virtual_live_id: Option<i64>,

    pub resource_box_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualLiveType {
    Archive,

    Beginner,

    #[serde(rename = "cheerful_carnival")]
    CheerfulCarnival,

    Normal,

    Paid,

    Streaming,

    #[serde(rename = "virtual_message")]
    VirtualMessage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveSchedule {
    pub id: Option<i64>,

    pub virtual_live_id: Option<i64>,

    pub seq: Option<i64>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,

    pub is_after_event: Option<bool>,

    pub notice_group_id: Option<NoticeGroupId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NoticeGroupId {
    #[serde(rename = "streamingLiveNoticeGroup_01")]
    StreamingLiveNoticeGroup01,

    #[serde(rename = "streamingLiveNoticeGroup_02")]
    StreamingLiveNoticeGroup02,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveSetlist {
    pub id: Option<i64>,

    pub virtual_live_id: Option<i64>,

    pub seq: Option<i64>,

    pub virtual_live_setlist_type: Option<VirtualLiveSetlistType>,

    pub assetbundle_name: Option<String>,

    pub virtual_live_stage_id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_vocal_id: Option<i64>,

    #[serde(rename = "character3dId1")]
    pub character3_d_id1: Option<i64>,

    #[serde(rename = "character3dId2")]
    pub character3_d_id2: Option<i64>,

    #[serde(rename = "character3dId3")]
    pub character3_d_id3: Option<i64>,

    #[serde(rename = "character3dId4")]
    pub character3_d_id4: Option<i64>,

    #[serde(rename = "character3dId5")]
    pub character3_d_id5: Option<i64>,

    #[serde(rename = "character3dId6")]
    pub character3_d_id6: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualLiveSetlistType {
    Mc,

    #[serde(rename = "mc_timeline")]
    McTimeline,

    Music,

    #[serde(rename = "virtual_message")]
    VirtualMessage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualLiveWaitingRoom {
    pub id: Option<i64>,

    pub virtual_live_id: Option<i64>,

    pub assetbundle_name: Option<AssetbundleName>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,

    pub lobby_assetbundle_name: Option<LobbyAssetbundleName>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssetbundleName {
    #[serde(rename = "banner_specialLive_sample_01")]
    BannerSpecialLiveSample01,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LobbyAssetbundleName {
    #[serde(rename = "new_year_2022")]
    NewYear2022,
}

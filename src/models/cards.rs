// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Card;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Card = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Card = Vec<CardElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub character_id: Option<i64>,

    pub card_rarity_type: Option<CardRarityType>,

    pub special_training_power1_bonus_fixed: Option<i64>,

    pub special_training_power2_bonus_fixed: Option<i64>,

    pub special_training_power3_bonus_fixed: Option<i64>,

    pub attr: Option<Attr>,

    pub support_unit: Option<SupportUnit>,

    pub skill_id: Option<i64>,

    pub card_skill_name: Option<String>,

    pub prefix: Option<String>,

    pub assetbundle_name: Option<String>,

    pub gacha_phrase: Option<String>,

    pub flavor_text:Option< Option<FlavorText>>,

    pub release_at: Option<i64>,

    pub archive_published_at: Option<i64>,

    pub card_supply_id: Option<i64>,

    pub card_parameters: Option<CardParametersUnion>,

    pub special_training_costs: Option<Vec<SpecialTrainingCost>>,

    pub master_lesson_achieve_resources: Option<Vec<MasterLessonAchieveResource>>,

    pub initial_special_training_status:Option< Option<InitialSpecialTrainingStatus>>,

    pub archive_display_type:Option< Option<ArchiveDisplayType>>,

    pub special_training_skill_id:Option< Option<i64>>,

    pub special_training_skill_name:Option< Option<String>>,

    pub special_training_reward_resource_box_id:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDisplayType {
    Hide,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attr {
    Cool,

    Cute,

    Happy,

    Mysterious,

    Pure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CardParametersUnion {
    CardParameterArray(Vec<CardParameter>),

    CardParametersClass(CardParametersClass),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardParameter {
    pub id: Option<i64>,

    pub card_id: Option<i64>,

    pub card_level: Option<i64>,

    pub card_parameter_type: Option<CardParameterType>,

    pub power: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardParameterType {
    Param1,

    Param2,

    Param3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardParametersClass {
    pub param1: Option<Vec<i64>>,

    pub param2: Option<Vec<i64>>,

    pub param3: Option<Vec<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardRarityType {
    #[serde(rename = "rarity_1")]
    Rarity1,

    #[serde(rename = "rarity_2")]
    Rarity2,

    #[serde(rename = "rarity_3")]
    Rarity3,

    #[serde(rename = "rarity_4")]
    Rarity4,

    #[serde(rename = "rarity_birthday")]
    RarityBirthday,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FlavorText {
    #[serde(rename = "フレーバーテキスト")]
    Empty,

    #[serde(rename = "-")]
    FlavorText,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InitialSpecialTrainingStatus {
    Done,

    #[serde(rename = "not_doing")]
    NotDoing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterLessonAchieveResource {
    pub release_condition_id:Option< Option<i64>>,

    pub card_id:Option< Option<i64>>,

    pub master_rank: Option<i64>,

    pub resources: Option<Vec<Option<serde_json::Value>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecialTrainingCost {
    pub card_id: Option<i64>,

    pub seq: Option<i64>,

    pub cost: Option<Cost>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub resource_id: Option<i64>,

    pub resource_type: Option<ResourceType>,

    pub resource_level: Option<i64>,

    pub quantity: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Material,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportUnit {
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

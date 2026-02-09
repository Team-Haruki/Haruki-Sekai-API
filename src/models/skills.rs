// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Skill;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Skill = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Skill = Vec<SkillElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillElement {
    pub id: Option<i64>,

    pub short_description:Option< Option<String>>,

    pub description: Option<String>,

    pub description_sprite_name: Option<DescriptionSpriteName>,

    pub skill_filter_id: Option<i64>,

    pub skill_effects: Option<Vec<SkillEffect>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptionSpriteName {
    #[serde(rename = "judgment_up")]
    JudgmentUp,

    #[serde(rename = "life_recovery")]
    LifeRecovery,

    #[serde(rename = "other_member_score_up_reference_rate")]
    OtherMemberScoreUpReferenceRate,

    #[serde(rename = "score_up")]
    ScoreUp,

    #[serde(rename = "score_up_character_rank")]
    ScoreUpCharacterRank,

    #[serde(rename = "score_up_condition_life")]
    ScoreUpConditionLife,

    #[serde(rename = "score_up_keep")]
    ScoreUpKeep,

    #[serde(rename = "score_up_unit_count")]
    ScoreUpUnitCount,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEffect {
    pub id: Option<i64>,

    pub skill_effect_type: Option<DescriptionSpriteName>,

    pub activate_notes_judgment_type:Option< Option<ActivateNotesJudgmentType>>,

    pub skill_effect_details: Option<Vec<SkillEffectDetail>>,

    pub activate_life:Option< Option<i64>>,

    pub condition_type:Option< Option<ConditionType>>,

    pub skill_enhance:Option< Option<SkillEnhance>>,

    pub activate_character_rank:Option< Option<i64>>,

    pub activate_unit_count:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivateNotesJudgmentType {
    Bad,

    Good,

    Great,

    Perfect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionType {
    #[serde(rename = "equals_or_over")]
    EqualsOrOver,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEffectDetail {
    pub id: Option<i64>,

    pub level: Option<i64>,

    pub activate_effect_duration: Option<f64>,

    pub activate_effect_value_type: Option<ActivateEffectValueType>,

    pub activate_effect_value: Option<i64>,

    pub activate_effect_value2:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivateEffectValueType {
    Fixed,

    Rate,

    #[serde(rename = "reference_rate")]
    ReferenceRate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEnhance {
    pub id: Option<i64>,

    pub skill_enhance_type: Option<SkillEnhanceType>,

    pub activate_effect_value_type: Option<ActivateEffectValueType>,

    pub activate_effect_value: Option<i64>,

    pub skill_enhance_condition: Option<SkillEnhanceCondition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillEnhanceCondition {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillEnhanceType {
    #[serde(rename = "sub_unit_score_up")]
    SubUnitScoreUp,
}

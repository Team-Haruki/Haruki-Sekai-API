// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Gacha;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Gacha = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Gacha = Vec<GachaElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaElement {
    pub id: Option<i64>,

    pub gacha_type: Option<GachaType>,

    pub name: Option<String>,

    pub seq: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub gacha_card_rarity_rate_group_id: Option<i64>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,

    pub is_show_period: Option<bool>,

    pub gacha_ceil_item_id: Option<i64>,

    pub wish_select_count: Option<i64>,

    pub wish_fixed_select_count: Option<i64>,

    pub wish_limited_select_count: Option<i64>,

    pub gacha_card_rarity_rates: Option<Vec<GachaCardRarityRate>>,

    pub gacha_details: Option<Vec<GachaDetail>>,

    pub gacha_behaviors: Option<Vec<GachaBehavior>>,

    pub gacha_pickups: Option<Vec<GachaPickup>>,

    pub gacha_pickup_costumes: Option<Vec<Option<serde_json::Value>>>,

    pub gacha_information: Option<GachaInformation>,

    pub drawable_gacha_hour: Option<i64>,

    pub gacha_bonus_id: Option<i64>,

    pub spin_limit: Option<i64>,

    pub gacha_bonus_item_receivable_reward_group_id: Option<i64>,

    pub gacha_freebie_group_id: Option<i64>,

    pub daily_spin_limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaBehavior {
    pub id: Option<i64>,

    pub gacha_id: Option<i64>,

    pub gacha_behavior_type: Option<GachaBehaviorType>,

    pub cost_resource_type: Option<CostResourceType>,

    pub cost_resource_quantity: Option<i64>,

    pub spin_count: Option<i64>,

    pub execute_limit: Option<i64>,

    pub group_id: Option<i64>,

    pub priority: Option<i64>,

    pub resource_category: Option<ResourceCategory>,

    pub gacha_spinnable_type: Option<GachaSpinnableType>,

    pub cost_resource_id: Option<i64>,

    pub gacha_extra_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostResourceType {
    #[serde(rename = "gacha_ticket")]
    GachaTicket,

    Jewel,

    #[serde(rename = "paid_jewel")]
    PaidJewel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GachaBehaviorType {
    Normal,

    #[serde(rename = "once_a_day")]
    OnceADay,

    #[serde(rename = "once_a_week")]
    OnceAWeek,

    #[serde(rename = "over_rarity_3_once")]
    OverRarity3Once,

    #[serde(rename = "over_rarity_4_once")]
    OverRarity4Once,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GachaSpinnableType {
    Any,

    #[serde(rename = "colorful_pass")]
    ColorfulPass,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceCategory {
    #[serde(rename = "consume_resource")]
    ConsumeResource,

    #[serde(rename = "free_resource")]
    FreeResource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaCardRarityRate {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub card_rarity_type: Option<CardRarityType>,

    pub lottery_type: Option<LotteryType>,

    pub rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardRarityType {
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
#[serde(rename_all = "snake_case")]
pub enum LotteryType {
    #[serde(rename = "categorized_wish")]
    CategorizedWish,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaDetail {
    pub id: Option<i64>,

    pub gacha_id: Option<i64>,

    pub card_id: Option<i64>,

    pub weight: Option<i64>,

    pub is_wish: Option<bool>,

    pub gacha_detail_wish_type: Option<GachaDetailWishType>,

    pub fixed_bonus_weight: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GachaDetailWishType {
    Fixed,

    Limited,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaInformation {
    pub gacha_id: Option<i64>,

    pub summary: Option<String>,

    pub description: Option<String>,

    pub bubble_assetbundle_name: Option<BubbleAssetbundleName>,

    pub bubble_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubbleAssetbundleName {
    #[serde(rename = "bubble_beginner")]
    BubbleBeginner,

    #[serde(rename = "bubble_birthday")]
    BubbleBirthday,

    #[serde(rename = "bubble_gift")]
    BubbleGift,

    #[serde(rename = "bubble_limit")]
    BubbleLimit,

    #[serde(rename = "bubble_normal")]
    BubbleNormal,

    #[serde(rename = "bubble_special")]
    BubbleSpecial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaPickup {
    pub id: Option<i64>,

    pub gacha_id: Option<i64>,

    pub card_id: Option<i64>,

    pub gacha_pickup_type: Option<GachaType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GachaType {
    Beginner,

    Ceil,

    Gift,

    Normal,
}

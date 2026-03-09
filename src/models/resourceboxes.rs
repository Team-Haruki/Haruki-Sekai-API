// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Resourceboxe;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Resourceboxe = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Resourceboxe = Vec<ResourceboxeElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceboxeElement {
    pub resource_box_purpose: Option<ResourceBoxPurpose>,

    pub id: Option<i64>,

    pub resource_box_type: Option<ResourceBoxType>,

    pub description: Option<String>,

    pub details: Option<Vec<Detail>>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub resource_box_purpose: Option<ResourceBoxPurpose>,

    pub resource_box_id: Option<i64>,

    pub seq: Option<i64>,

    pub resource_type: Option<ResourceType>,

    pub resource_id: Option<i64>,

    pub resource_quantity: Option<i64>,

    pub resource_level: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceBoxPurpose {
    #[serde(rename = "ad_reward")]
    AdReward,

    #[serde(rename = "ad_reward_random_box")]
    AdRewardRandomBox,

    #[serde(rename = "billing_shop_item")]
    BillingShopItem,

    #[serde(rename = "billing_shop_item_bonus")]
    BillingShopItemBonus,

    #[serde(rename = "billing_shop_item_count_bonus")]
    BillingShopItemCountBonus,

    #[serde(rename = "billing_shop_item_zenpay_bonus")]
    BillingShopItemZenpayBonus,

    #[serde(rename = "birthday_party_delivery_total_reward")]
    BirthdayPartyDeliveryTotalReward,

    #[serde(rename = "bonds_reward")]
    BondsReward,

    #[serde(rename = "card_exchange_resource")]
    CardExchangeResource,

    #[serde(rename = "card_extra")]
    CardExtra,

    #[serde(rename = "challenge_live_high_score")]
    ChallengeLiveHighScore,

    #[serde(rename = "challenge_live_play_day_reward")]
    ChallengeLivePlayDayReward,

    #[serde(rename = "challenge_live_score_rank_reward_detail")]
    ChallengeLiveScoreRankRewardDetail,

    #[serde(rename = "challenge_live_stage")]
    ChallengeLiveStage,

    #[serde(rename = "challenge_live_stage_ex")]
    ChallengeLiveStageEx,

    #[serde(rename = "character_rank_reward")]
    CharacterRankReward,

    #[serde(rename = "cheerful_carnival_result_reward")]
    CheerfulCarnivalResultReward,

    #[serde(rename = "cheerful_carnival_reward")]
    CheerfulCarnivalReward,

    #[serde(rename = "colorful_pass")]
    ColorfulPass,

    #[serde(rename = "colorful_pass_v2")]
    ColorfulPassV2,

    Compensation,

    #[serde(rename = "connect_live_reward")]
    ConnectLiveReward,

    #[serde(rename = "convert_gacha_ceil_item")]
    ConvertGachaCeilItem,

    #[serde(rename = "douyin_gift")]
    DouyinGift,

    #[serde(rename = "episode_reward")]
    EpisodeReward,

    #[serde(rename = "event_exchange")]
    EventExchange,

    #[serde(rename = "event_mission_selectable_reward")]
    EventMissionSelectableReward,

    #[serde(rename = "event_ranking_reward")]
    EventRankingReward,

    #[serde(rename = "friend_invitation_campaign_mission_reward")]
    FriendInvitationCampaignMissionReward,

    #[serde(rename = "gacha_bonus_item_receivable_reward")]
    GachaBonusItemReceivableReward,

    #[serde(rename = "gacha_ceil_exchange")]
    GachaCeilExchange,

    #[serde(rename = "gacha_extra")]
    GachaExtra,

    #[serde(rename = "gacha_freebie_group")]
    GachaFreebieGroup,

    #[serde(rename = "gift_detail")]
    GiftDetail,

    #[serde(rename = "gift_gacha_extra")]
    GiftGachaExtra,

    #[serde(rename = "limited_term_score_rank_reward_detail")]
    LimitedTermScoreRankRewardDetail,

    #[serde(rename = "login_bonus")]
    LoginBonus,

    #[serde(rename = "master_lesson_reward")]
    MasterLessonReward,

    #[serde(rename = "material_exchange")]
    MaterialExchange,

    #[serde(rename = "material_exchange_freebie")]
    MaterialExchangeFreebie,

    #[serde(rename = "mission_reward")]
    MissionReward,

    #[serde(rename = "multi_score_rank_reward_detail")]
    MultiScoreRankRewardDetail,

    #[serde(rename = "music_achievement")]
    MusicAchievement,

    #[serde(rename = "mysekai_convert_consumption")]
    MysekaiConvertConsumption,

    #[serde(rename = "mysekai_convert_obtain")]
    MysekaiConvertObtain,

    #[serde(rename = "mysekai_fixture_disassemble_resource")]
    MysekaiFixtureDisassembleResource,

    #[serde(rename = "mysekai_housing_competition_back_number_reward")]
    MysekaiHousingCompetitionBackNumberReward,

    #[serde(rename = "mysekai_normal_mission_reward")]
    MysekaiNormalMissionReward,

    #[serde(rename = "mysekai_recycle")]
    MysekaiRecycle,

    #[serde(rename = "paid_virtual_live_shop_item")]
    PaidVirtualLiveShopItem,

    #[serde(rename = "player_rank_reward")]
    PlayerRankReward,

    #[serde(rename = "rank_match_score_rank_reward_detail")]
    RankMatchScoreRankRewardDetail,

    #[serde(rename = "rank_match_season_tier_reward")]
    RankMatchSeasonTierReward,

    #[serde(rename = "recharge_reward")]
    RechargeReward,

    #[serde(rename = "score_rank_reward_detail")]
    ScoreRankRewardDetail,

    #[serde(rename = "serial_code_campaign_reward")]
    SerialCodeCampaignReward,

    #[serde(rename = "shining_exchange")]
    ShiningExchange,

    #[serde(rename = "shop_item")]
    ShopItem,

    #[serde(rename = "special_reward")]
    SpecialReward,

    #[serde(rename = "special_training_reward")]
    SpecialTrainingReward,

    #[serde(rename = "spring_event_ranking_reward")]
    SpringEventRankingReward,

    #[serde(rename = "spring_event_unit_reward")]
    SpringEventUnitReward,

    #[serde(rename = "story_mission")]
    StoryMission,

    #[serde(rename = "super_fever_reward")]
    SuperFeverReward,

    #[serde(rename = "support_event_personal_reward")]
    SupportEventPersonalReward,

    #[serde(rename = "support_event_total_reward")]
    SupportEventTotalReward,

    #[serde(rename = "virtual_live_cheer_point_reward")]
    VirtualLiveCheerPointReward,

    #[serde(rename = "virtual_live_member_count_reward")]
    VirtualLiveMemberCountReward,

    #[serde(rename = "virtual_live_reward")]
    VirtualLiveReward,

    #[serde(rename = "virtual_shop_bonus_item")]
    VirtualShopBonusItem,

    #[serde(rename = "virtual_shop_item")]
    VirtualShopItem,

    #[serde(rename = "virtual_shop_item_first_purchase_bonus")]
    VirtualShopItemFirstPurchaseBonus,

    #[serde(rename = "virtual_shop_item_first_purchase_bonus_bonus")]
    VirtualShopItemFirstPurchaseBonusBonus,

    #[serde(rename = "world_bloom_chapter_ranking_reward")]
    WorldBloomChapterRankingReward,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    #[serde(rename = "ad_reward_random_box")]
    AdRewardRandomBox,

    #[serde(rename = "area_item")]
    AreaItem,

    #[serde(rename = "avatar_accessory")]
    AvatarAccessory,

    #[serde(rename = "avatar_coordinate")]
    AvatarCoordinate,

    #[serde(rename = "avatar_costume")]
    AvatarCostume,

    #[serde(rename = "avatar_motion")]
    AvatarMotion,

    #[serde(rename = "avatar_skin_color")]
    AvatarSkinColor,

    #[serde(rename = "bonds_honor")]
    BondsHonor,

    #[serde(rename = "bonds_honor_word")]
    BondsHonorWord,

    #[serde(rename = "boost_item")]
    BoostItem,

    Card,

    #[serde(rename = "character_rank_exp")]
    CharacterRankExp,

    Coin,

    #[serde(rename = "colorful_pass")]
    ColorfulPass,

    #[serde(rename = "colorful_pass_v2")]
    ColorfulPassV2,

    #[serde(rename = "costume_3d")]
    Costume3D,

    #[serde(rename = "custom_profile_collection_resource")]
    CustomProfileCollectionResource,

    #[serde(rename = "event_item")]
    EventItem,

    #[serde(rename = "gacha_ceil_item")]
    GachaCeilItem,

    #[serde(rename = "gacha_ticket")]
    GachaTicket,

    Honor,

    Jewel,

    #[serde(rename = "live_point")]
    LivePoint,

    Material,

    Music,

    #[serde(rename = "music_vocal")]
    MusicVocal,

    #[serde(rename = "mysekai_blueprint")]
    MysekaiBlueprint,

    #[serde(rename = "mysekai_colorful_pass")]
    MysekaiColorfulPass,

    #[serde(rename = "mysekai_fixture")]
    MysekaiFixture,

    #[serde(rename = "mysekai_item")]
    MysekaiItem,

    #[serde(rename = "mysekai_material")]
    MysekaiMaterial,

    #[serde(rename = "mysekai_tool")]
    MysekaiTool,

    #[serde(rename = "paid_jewel")]
    PaidJewel,

    #[serde(rename = "paid_virtual_live")]
    PaidVirtualLive,

    Penlight,

    #[serde(rename = "player_frame")]
    PlayerFrame,

    #[serde(rename = "practice_ticket")]
    PracticeTicket,

    #[serde(rename = "serial_code_item")]
    SerialCodeItem,

    #[serde(rename = "skill_practice_ticket")]
    SkillPracticeTicket,

    Stamp,

    #[serde(rename = "virtual_coin")]
    VirtualCoin,

    #[serde(rename = "virtual_live_pamphlet")]
    VirtualLivePamphlet,

    #[serde(rename = "virtual_live_ticket")]
    VirtualLiveTicket,

    #[serde(rename = "virtual_live_transition_item")]
    VirtualLiveTransitionItem,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceBoxType {
    #[serde(rename = "costume_3d")]
    Costume3D,

    Expand,

    List,
}

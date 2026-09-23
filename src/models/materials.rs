// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Material;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Material = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Material = Vec<MaterialElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub name: Option<String>,

    pub flavor_text: Option<String>,

    pub can_use: Option<bool>,

    pub material_type: Option<MaterialType>,

    pub flavor_text2: Option<String>,

    pub change_flavor_text_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialType {
    #[serde(rename = "auto_exchange_music_vocal_ticket")]
    AutoExchangeMusicVocalTicket,

    #[serde(rename = "birthday_party_delivery")]
    BirthdayPartyDelivery,

    #[serde(rename = "card_episode_release_ticket")]
    CardEpisodeReleaseTicket,

    #[serde(rename = "card_ticket")]
    CardTicket,

    #[serde(rename = "character_rank_exp_ticket")]
    CharacterRankExpTicket,

    Common,

    Costume,

    #[serde(rename = "gacha_ceil_ticket")]
    GachaCeilTicket,

    #[serde(rename = "master_lesson")]
    MasterLesson,

    Music,

    #[serde(rename = "special_training")]
    SpecialTraining,

    #[serde(rename = "vocal_card_ticket")]
    VocalCardTicket,

    #[serde(rename = "web_ticket")]
    WebTicket,
}

// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaicharactertalk;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaicharactertalk = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaicharactertalk = Vec<MysekaicharactertalkElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaicharactertalkElement {
    pub id: Option<i64>,

    pub mysekai_game_character_unit_group_id: Option<i64>,

    pub mysekai_character_talk_condition_group_id: Option<i64>,

    pub mysekai_site_group_id: Option<i64>,

    pub mysekai_character_talk_term_id: Option<i64>,

    pub character_archive_mysekai_character_talk_group_id: Option<i64>,

    pub assetbundle_name: Option<AssetbundleName>,

    pub lua: Option<String>,

    pub is_enabled_for_multi: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssetbundleName {
    #[serde(rename = "mysekai/talk/scenario/talk")]
    MysekaiTalkScenarioTalk,
}

// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaimaterial;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaimaterial = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaimaterial = Vec<MysekaimaterialElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaimaterialElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub mysekai_material_type: Option<MysekaiMaterialType>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,

    pub description: Option<String>,

    pub mysekai_material_rarity_type: Option<MysekaiMaterialRarityType>,

    pub icon_assetbundle_name: Option<String>,

    pub model_assetbundle_name:Option<String>,

    pub mysekai_site_ids: Option<Vec<i64>>,

    pub mysekai_phenomena_group_id:Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiMaterialRarityType {
    #[serde(rename = "rarity_1")]
    Rarity1,

    #[serde(rename = "rarity_2")]
    Rarity2,

    #[serde(rename = "rarity_3")]
    Rarity3,

    #[serde(rename = "rarity_4")]
    Rarity4,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiMaterialType {
    #[serde(rename = "birthday_party")]
    BirthdayParty,

    #[serde(rename = "game_character")]
    GameCharacter,

    Junk,

    Mineral,

    Plant,

    Tone,

    Wood,
}

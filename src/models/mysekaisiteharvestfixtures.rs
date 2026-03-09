// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaisiteharvestfixture;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaisiteharvestfixture = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaisiteharvestfixture = Vec<MysekaisiteharvestfixtureElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaisiteharvestfixtureElement {
    pub id: Option<i64>,

    pub mysekai_site_harvest_fixture_type: Option<String>,

    pub hp: Option<i64>,

    pub last_attack_stamina: Option<i64>,

    pub mysekai_site_harvest_fixture_rarity_type: Option<MysekaiSiteHarvestFixtureRarityType>,

    pub assetbundle_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiSiteHarvestFixtureRarityType {
    #[serde(rename = "rarity_1")]
    Rarity1,

    #[serde(rename = "rarity_2")]
    Rarity2,
}

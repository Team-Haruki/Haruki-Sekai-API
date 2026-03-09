// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaiphenomena;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaiphenomena = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaiphenomena = Vec<Mysekaiphenomenon>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mysekaiphenomenon {
    pub id: Option<i64>,

    pub mysekai_phenomena_brightness_type: Option<MysekaiPhenomenaBrightnessType>,

    pub name: Option<String>,

    pub english_name: Option<String>,

    pub description: Option<String>,

    pub mysekai_phenomena_time_period_type: Option<MysekaiPhenomenaTimePeriodType>,

    pub mysekai_phenomena_background_color_id: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub ramp_texture_assetbundle_name: Option<String>,

    pub icon_assetbundle_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiPhenomenaBrightnessType {
    Bright,

    Dark,

    None,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiPhenomenaTimePeriodType {
    Daytime,

    Evening,

    Night,
}

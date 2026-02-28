// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Boostitem;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Boostitem = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Boostitem = Vec<BoostitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoostitemElement {
    pub id: Option<i64>,

    pub seq:Option<i64>,

    pub name: Option<String>,

    pub recovery_value: Option<i64>,

    pub asset_bundle_name:Option<String>,

    pub flavor_text: Option<FlavorText>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FlavorText {
    #[serde(rename = "恢复1演出能量。")]
    FlavorText1,

    #[serde(rename = "ライブボーナスを10回復する。\n※テストイベント終了後、メンテナンスにて回収します。")]
    FlavorText10,

    #[serde(rename = "恢复99演出能量。")]
    FlavorText99,

    #[serde(rename = "恢复10演出能量。")]
    Purple10,

    #[serde(rename = "ライブボーナスを1回復する。")]
    The1,

    #[serde(rename = "ライブボーナスを10回復する。")]
    The10,

    #[serde(rename = "ライブボーナスを99回復する。")]
    The99,
}

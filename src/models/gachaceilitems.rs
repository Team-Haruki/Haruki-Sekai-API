// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Gachaceilitem;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Gachaceilitem = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Gachaceilitem = Vec<GachaceilitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GachaceilitemElement {
    pub id: Option<i64>,

    pub gacha_id: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<AssetbundleName>,

    pub convert_start_at: Option<i64>,

    pub convert_resource_box_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetbundleName {
    #[serde(rename = "ceil_item")]
    CeilItem,

    #[serde(rename = "ceil_item_birthday")]
    CeilItemBirthday,

    #[serde(rename = "ceil_item_limited")]
    CeilItemLimited,
}

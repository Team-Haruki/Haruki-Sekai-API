// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Areaitem;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Areaitem = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Areaitem = Vec<AreaitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaitemElement {
    pub id: Option<i64>,

    pub area_id: Option<i64>,

    pub name: Option<String>,

    pub flavor_text:Option< Option<String>>,

    pub spawn_point: Option<SpawnPoint>,

    pub assetbundle_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpawnPoint {
    #[serde(rename = "spawn-point1")]
    SpawnPoint1,

    #[serde(rename = "spawn-point10")]
    SpawnPoint10,

    #[serde(rename = "spawn-point2")]
    SpawnPoint2,

    #[serde(rename = "spawn-point3")]
    SpawnPoint3,

    #[serde(rename = "spawn-point4")]
    SpawnPoint4,

    #[serde(rename = "spawn-point5")]
    SpawnPoint5,

    #[serde(rename = "spawn-point6")]
    SpawnPoint6,

    #[serde(rename = "spawn-point7")]
    SpawnPoint7,

    #[serde(rename = "spawn-point8")]
    SpawnPoint8,

    #[serde(rename = "spawn-point9")]
    SpawnPoint9,
}

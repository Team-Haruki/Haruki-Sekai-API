// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Cheerfulcarnivalteam;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Cheerfulcarnivalteam = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Cheerfulcarnivalteam = Vec<CheerfulcarnivalteamElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheerfulcarnivalteamElement {
    pub id: Option<i64>,

    pub event_id: Option<i64>,

    pub seq: Option<i64>,

    pub team_name: Option<String>,

    pub assetbundle_name: Option<String>,
}

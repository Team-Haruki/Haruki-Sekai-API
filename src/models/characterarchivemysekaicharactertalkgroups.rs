// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Characterarchivemysekaicharactertalkgroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Characterarchivemysekaicharactertalkgroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Characterarchivemysekaicharactertalkgroup =
    Vec<CharacterarchivemysekaicharactertalkgroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterarchivemysekaicharactertalkgroupElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub archive_display_type: Option<ArchiveDisplayType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDisplayType {
    Hide,

    Normal,
}

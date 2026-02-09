// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaigamecharacterunitgroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaigamecharacterunitgroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaigamecharacterunitgroup = Vec<MysekaigamecharacterunitgroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigamecharacterunitgroupElement {
    pub id: Option<i64>,

    pub game_character_unit_id1: Option<i64>,

    pub game_character_unit_id2:Option< Option<i64>>,

    pub game_character_unit_id3:Option< Option<i64>>,

    pub game_character_unit_id4:Option< Option<i64>>,

    pub game_character_unit_id5:Option< Option<i64>>,
}

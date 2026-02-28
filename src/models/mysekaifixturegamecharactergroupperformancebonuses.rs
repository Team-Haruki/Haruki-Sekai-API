// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaifixturegamecharactergroupperformancebonuse;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaifixturegamecharactergroupperformancebonuse = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaifixturegamecharactergroupperformancebonuse = Vec<MysekaifixturegamecharactergroupperformancebonuseElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaifixturegamecharactergroupperformancebonuseElement {
    pub id: Option<i64>,

    pub mysekai_fixture_game_character_group_id: Option<i64>,

    pub bonus_rate: Option<i64>,
}

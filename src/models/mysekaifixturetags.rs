// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaifixturetag;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaifixturetag = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaifixturetag = Vec<MysekaifixturetagElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaifixturetagElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,

    pub mysekai_fixture_tag_type: Option<MysekaiFixtureTagType>,

    pub external_id:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiFixtureTagType {
    #[serde(rename = "game_character")]
    GameCharacter,

    None,

    Series,

    Unit,
}

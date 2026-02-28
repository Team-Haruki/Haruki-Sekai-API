// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaifixturegamecharactergroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaifixturegamecharactergroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaifixturegamecharactergroup = Vec<MysekaifixturegamecharactergroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaifixturegamecharactergroupElement {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub game_character_id: Option<i64>,
}

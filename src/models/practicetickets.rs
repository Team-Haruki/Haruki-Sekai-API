// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Practiceticket;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Practiceticket = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Practiceticket = Vec<PracticeticketElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeticketElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub exp: Option<i64>,

    pub flavor_text: Option<String>,

    pub character_id: Option<i64>,
}

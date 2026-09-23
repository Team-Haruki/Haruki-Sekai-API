// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Skillpracticeticket;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Skillpracticeticket = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Skillpracticeticket = Vec<SkillpracticeticketElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillpracticeticketElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub exp: Option<i64>,

    pub flavor_text: Option<String>,

    pub character_id: Option<i64>,
}

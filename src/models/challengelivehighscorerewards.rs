// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Challengelivehighscorereward;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Challengelivehighscorereward = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Challengelivehighscorereward = Vec<ChallengelivehighscorerewardElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengelivehighscorerewardElement {
    pub id: Option<i64>,

    pub character_id: Option<i64>,

    pub high_score: Option<i64>,

    pub resource_box_id: Option<i64>,
}

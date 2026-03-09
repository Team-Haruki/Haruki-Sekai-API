// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaicharactertalkfixturecommonmysekaifixturegroup;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaicharactertalkfixturecommonmysekaifixturegroup = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaicharactertalkfixturecommonmysekaifixturegroup =
    Vec<MysekaicharactertalkfixturecommonmysekaifixturegroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaicharactertalkfixturecommonmysekaifixturegroupElement {
    pub id: Option<i64>,

    pub group_id: Option<i64>,

    pub mysekai_fixture_id: Option<i64>,
}

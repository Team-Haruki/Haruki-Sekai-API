// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaifixturesubgenre;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaifixturesubgenre = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaifixturesubgenre = Vec<MysekaifixturesubgenreElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaifixturesubgenreElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub mysekai_fixture_sub_genre_type: Option<String>,

    pub assetbundle_name: Option<String>,
}

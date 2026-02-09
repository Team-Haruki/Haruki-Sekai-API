// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaifixturemaingenre;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaifixturemaingenre = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Mysekaifixturemaingenre = Vec<MysekaifixturemaingenreElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaifixturemaingenreElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub mysekai_fixture_main_genre_type: Option<MysekaiFixtureMainGenreType>,

    pub assetbundle_name: Option<String>,

    pub group_id:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiFixtureMainGenreType {
    Fence,

    Home,

    None,

    Road,
}

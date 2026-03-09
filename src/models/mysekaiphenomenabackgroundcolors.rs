// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaiphenomenabackgroundcolor;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaiphenomenabackgroundcolor = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaiphenomenabackgroundcolor = Vec<MysekaiphenomenabackgroundcolorElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiphenomenabackgroundcolorElement {
    pub id: Option<i64>,

    pub base_color: Option<String>,

    pub ground_color: Option<String>,

    pub gradation_color: Option<String>,

    pub corner_color: Option<String>,

    pub ground_highlight_color: Option<String>,
}

// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Cardcostume3D;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Cardcostume3D = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Cardcostume3D = Vec<Cardcostume3DElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cardcostume3DElement {
    pub card_id: Option<i64>,

    #[serde(rename = "costume3dId")]
    pub costume3_d_id: Option<i64>,

    pub is_initial_obtain_hair: Option<bool>,
}

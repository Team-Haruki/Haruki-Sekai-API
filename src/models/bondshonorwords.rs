// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Bondshonorword;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Bondshonorword = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Bondshonorword = Vec<BondshonorwordElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BondshonorwordElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub bonds_group_id: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub name: Option<String>,

    pub description: Option<String>,
}

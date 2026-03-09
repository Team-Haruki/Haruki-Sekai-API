// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Limitedtimemusic;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Limitedtimemusic = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Limitedtimemusic = Vec<LimitedtimemusicElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitedtimemusicElement {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,
}

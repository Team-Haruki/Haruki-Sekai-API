// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Eventexchangesummarie;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Eventexchangesummarie = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Eventexchangesummarie = Vec<EventexchangesummarieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventexchangesummarieElement {
    pub id: Option<i64>,

    pub event_id: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,

    pub event_exchanges: Option<Vec<EventExchange>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventExchange {
    pub id: Option<i64>,

    pub event_exchange_summary_id: Option<i64>,

    pub seq: Option<i64>,

    pub resource_box_id: Option<i64>,

    pub exchange_limit:Option< Option<i64>>,

    pub event_exchange_cost: Option<EventExchangeCost>,

    pub game_character_id:Option< Option<i64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventExchangeCost {
    pub id:Option< Option<i64>>,

    pub event_exchange_id:Option< Option<i64>>,

    pub resource_type:Option< Option<ResourceType>>,

    pub resource_id:Option< Option<i64>>,

    pub resource_quantity: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    #[serde(rename = "event_item")]
    EventItem,
}

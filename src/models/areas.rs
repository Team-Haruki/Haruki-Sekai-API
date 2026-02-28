// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Area;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Area = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Area = Vec<AreaElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaElement {
    pub id: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub group_id: Option<i64>,

    pub is_base_area: Option<bool>,

    pub area_type: Option<AreaType>,

    pub view_type: Option<ViewType>,

    pub display_timeline_type: Option<DisplayTimelineType>,

    pub additional_area_type: Option<AdditionalAreaType>,

    pub name: Option<String>,

    pub release_condition_id: Option<i64>,

    pub sub_name:Option<String>,

    pub label:Option<String>,

    pub start_at:Option<i64>,

    pub end_at:Option<i64>,

    pub release_condition_id2:Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdditionalAreaType {
    #[serde(rename = "april_fool")]
    AprilFool,

    #[serde(rename = "center_of_ring")]
    CenterOfRing,

    Collaboration,

    None,

    #[serde(rename = "out_of_ring")]
    OutOfRing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaType {
    #[serde(rename = "reality_world")]
    RealityWorld,

    #[serde(rename = "spirit_world")]
    SpiritWorld,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayTimelineType {
    All,

    #[serde(rename = "next_grade")]
    NextGrade,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewType {
    #[serde(rename = "quarter_view")]
    QuarterView,

    #[serde(rename = "side_view")]
    SideView,
}

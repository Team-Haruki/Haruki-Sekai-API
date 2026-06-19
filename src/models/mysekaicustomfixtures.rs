use serde::{Deserialize, Serialize};

pub type Mysekaicustomfixture = Vec<MysekaicustomfixtureElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaicustomfixtureElement {
    pub id: Option<i64>,

    pub mysekai_fixture_id: Option<i64>,

    pub mysekai_custom_fixture_type: Option<String>,

    pub mysekai_custom_fixture_ornament_type: Option<String>,

    pub custom_profile_resource_collection_type: Option<String>,

    pub width: Option<i64>,

    pub height: Option<i64>,

    pub depth: Option<i64>,

    pub base_asset_bundle_name: Option<String>,

    pub ornament_asset_bundle_name: Option<String>,
}

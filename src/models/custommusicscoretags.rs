use serde::{Deserialize, Serialize};

pub type Custommusicscoretag = Vec<CustommusicscoretagElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustommusicscoretagElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub name: Option<String>,

    pub is_official_creator_only: Option<bool>,
}

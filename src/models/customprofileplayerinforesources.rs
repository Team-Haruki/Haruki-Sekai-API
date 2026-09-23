use serde::{Deserialize, Serialize};

pub type Customprofileplayerinforesource = Vec<CustomprofileplayerinforesourceElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomprofileplayerinforesourceElement {
    pub custom_profile_resource_type: Option<String>,

    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,

    pub resource_load_type: Option<String>,

    pub resource_load_val: Option<String>,

    pub file_name: Option<String>,

    pub group_id: Option<i64>,
}

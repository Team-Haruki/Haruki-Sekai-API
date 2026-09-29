use serde::{Deserialize, Serialize};

pub type Virtuallivegroup = Vec<VirtuallivegroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtuallivegroupElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub virtual_live_group_type: Option<String>,

    pub assetbundle_name: Option<String>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,
}

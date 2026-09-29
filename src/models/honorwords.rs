use serde::{Deserialize, Serialize};

pub type Honorword = Vec<HonorwordElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorwordElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub honor_group_id: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub name: Option<String>,

    pub description: Option<String>,
}

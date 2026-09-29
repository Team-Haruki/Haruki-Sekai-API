use serde::{Deserialize, Serialize};

pub type Honorbackground = Vec<HonorbackgroundElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HonorbackgroundElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub honor_group_id: Option<i64>,

    pub assetbundle_name: Option<String>,

    pub name: Option<String>,

    pub description: Option<String>,
}

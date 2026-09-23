use serde::{Deserialize, Serialize};

pub type Unitstoryepisodegroup = Vec<UnitstoryepisodegroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitstoryepisodegroupElement {
    pub id: Option<i64>,

    pub unit: Option<String>,

    pub unit_episode_category: Option<String>,

    pub outline: Option<String>,

    pub assetbundle_name: Option<String>,
}

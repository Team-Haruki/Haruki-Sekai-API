use serde::{Deserialize, Serialize};

pub type Mysekairankrelease = Vec<MysekairankreleaseElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekairankreleaseElement {
    pub id: Option<i64>,

    pub mysekai_rank: Option<i64>,

    pub mysekai_rank_relase_type: Option<String>,

    pub external_id: Option<i64>,
}

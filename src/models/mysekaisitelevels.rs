use serde::{Deserialize, Serialize};

pub type Mysekaisitelevel = Vec<MysekaisitelevelElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaisitelevelElement {
    pub id: Option<i64>,

    pub mysekai_site_id: Option<i64>,

    pub level: Option<i64>,

    pub character_entry_max_num: Option<i64>,
}

use serde::{Deserialize, Serialize};

pub type Mysekaisitelayout = Vec<MysekaisitelayoutElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaisitelayoutElement {
    pub id: Option<i64>,

    pub mysekai_site_level_id: Option<i64>,

    pub mysekai_layout_type: Option<String>,

    pub width: Option<i64>,

    pub height: Option<i64>,

    pub depth: Option<i64>,
}

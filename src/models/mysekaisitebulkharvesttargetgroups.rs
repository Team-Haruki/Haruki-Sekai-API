use serde::{Deserialize, Serialize};

pub type Mysekaisitebulkharvesttargetgroup = Vec<MysekaisitebulkharvesttargetgroupElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaisitebulkharvesttargetgroupElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub name: Option<String>,

    pub required_tool_id: Option<i64>,
}

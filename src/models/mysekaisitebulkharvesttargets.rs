use serde::{Deserialize, Serialize};

pub type Mysekaisitebulkharvesttarget = Vec<MysekaisitebulkharvesttargetElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaisitebulkharvesttargetElement {
    pub id: Option<i64>,

    pub mysekai_site_bulk_harvest_target_group_id: Option<i64>,

    pub seq: Option<i64>,

    pub name: Option<String>,
}

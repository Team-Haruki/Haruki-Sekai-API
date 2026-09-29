use serde::{Deserialize, Serialize};

pub type Mysekaisitebulkharvest = Vec<MysekaisitebulkharvestElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaisitebulkharvestElement {
    pub id: Option<i64>,

    pub mysekai_site_id: Option<i64>,

    pub mysekai_site_bulk_harvest_target_id: Option<i64>,
}

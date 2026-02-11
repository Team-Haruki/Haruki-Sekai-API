//! RegionalNgWords - Region-wrapped NG word data
//! Not merged by ID — each region has independent word lists.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::ngwords::NgwordElement;

use super::types::RegionalData;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionalNgWords {
    #[serde(flatten)]
    pub data: RegionalData<Vec<NgwordElement>>,
}

pub fn wrap_ng_words(
    region_data: std::collections::HashMap<ServerRegion, Vec<NgwordElement>>,
) -> RegionalNgWords {
    let mut data = RegionalData::new();
    for (region, items) in region_data {
        data.set(region, items);
    }
    RegionalNgWords { data }
}

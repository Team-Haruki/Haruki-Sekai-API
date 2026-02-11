//! RegionalLimitedTimeMusic - Region-wrapped limited time music data
//! Not merged by ID — each region has independent scheduling data.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::limitedtimemusics::LimitedtimemusicElement;

use super::types::RegionalData;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionalLimitedTimeMusics {
    #[serde(flatten)]
    pub data: RegionalData<Vec<LimitedtimemusicElement>>,
}

pub fn wrap_limited_time_musics(
    region_data: std::collections::HashMap<ServerRegion, Vec<LimitedtimemusicElement>>,
) -> RegionalLimitedTimeMusics {
    let mut data = RegionalData::new();
    for (region, items) in region_data {
        data.set(region, items);
    }
    RegionalLimitedTimeMusics { data }
}

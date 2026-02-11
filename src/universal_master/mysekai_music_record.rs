//! UniversalMysekaiMusicRecord - Merged music record data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaimusicrecords::MysekaimusicrecordElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaimusicrecordElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiMusicRecord {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_music_track_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiMusicRecord {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaimusicrecordElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiMusicRecord {
            id,
            mysekai_music_track_type: get_first_value(regional, |m| {
                m.mysekai_music_track_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            external_id: get_first_value(regional, |m| m.external_id),
            available_regions,
        })
    }
}

pub fn merge_mysekai_music_records(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaimusicrecordElement>>,
) -> Vec<UniversalMysekaiMusicRecord> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiMusicRecord> = by_id
        .values()
        .filter_map(UniversalMysekaiMusicRecord::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

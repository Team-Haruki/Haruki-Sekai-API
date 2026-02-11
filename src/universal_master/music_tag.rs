//! UniversalMusicTag - Merged music tag data across all regions
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::musictags::MusictagElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MusictagElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.flatten().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMusicTag {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_tag: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMusicTag {
    pub fn from_regional(regional: &super::types::RegionalData<MusictagElement>) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id.flatten())?;
        let available_regions = regional.available_regions();

        Some(UniversalMusicTag {
            id,
            music_id: get_first_value(regional, |m| m.music_id),
            music_tag: get_first_value(regional, |m| {
                m.music_tag.as_ref().map(|t| format!("{:?}", t))
            }),
            seq: get_first_value(regional, |m| m.seq.flatten()),
            available_regions,
        })
    }
}

pub fn merge_music_tags(
    region_data: std::collections::HashMap<ServerRegion, Vec<MusictagElement>>,
) -> Vec<UniversalMusicTag> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMusicTag> = by_id
        .values()
        .filter_map(UniversalMusicTag::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

//! UniversalCheerfulCarnivalTeam - Mixed: team_name is regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::cheerfulcarnivalteams::CheerfulcarnivalteamElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for CheerfulcarnivalteamElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCheerfulCarnivalTeam {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    pub team_name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCheerfulCarnivalTeam {
    pub fn from_regional(
        regional: &super::types::RegionalData<CheerfulcarnivalteamElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCheerfulCarnivalTeam {
            id,
            event_id: get_first_value(regional, |c| c.event_id),
            seq: get_first_value(regional, |c| c.seq),
            team_name: merge_field(regional, |c| c.team_name.clone()),
            assetbundle_name: get_first_value(regional, |c| c.assetbundle_name.clone()),
            available_regions,
        })
    }
}

pub fn merge_cheerful_carnival_teams(
    region_data: std::collections::HashMap<ServerRegion, Vec<CheerfulcarnivalteamElement>>,
) -> Vec<UniversalCheerfulCarnivalTeam> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCheerfulCarnivalTeam> = by_id
        .values()
        .filter_map(UniversalCheerfulCarnivalTeam::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

//! UniversalCharacterMissionV2ParameterGroup - Merged character mission parameter data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::charactermissionv2parametergroups::Charactermissionv2ParametergroupElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for Charactermissionv2ParametergroupElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCharacterMissionV2ParameterGroup {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCharacterMissionV2ParameterGroup {
    pub fn from_regional(
        regional: &super::types::RegionalData<Charactermissionv2ParametergroupElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalCharacterMissionV2ParameterGroup {
            id,
            seq: get_first_value(regional, |c| c.seq),
            requirement: get_first_value(regional, |c| c.requirement),
            exp: get_first_value(regional, |c| c.exp),
            quantity: get_first_value(regional, |c| c.quantity),
            available_regions,
        })
    }
}

pub fn merge_character_mission_v2_parameter_groups(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<Charactermissionv2ParametergroupElement>,
    >,
) -> Vec<UniversalCharacterMissionV2ParameterGroup> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCharacterMissionV2ParameterGroup> = by_id
        .values()
        .filter_map(UniversalCharacterMissionV2ParameterGroup::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

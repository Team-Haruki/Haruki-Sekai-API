//! UniversalOutsideCharacter - Merged outside character data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::outsidecharacters::OutsidecharacterElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for OutsidecharacterElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalOutsideCharacter {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    // Regional field
    pub name: UnifiedValue<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalOutsideCharacter {
    pub fn from_regional(
        regional: &super::types::RegionalData<OutsidecharacterElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |o| o.id)?;
        let available_regions = regional.available_regions();

        let seq = get_first_value(regional, |o| o.seq.flatten());
        let name = merge_field(regional, |o| o.name.clone());

        Some(UniversalOutsideCharacter {
            id,
            seq,
            name,
            available_regions,
        })
    }
}

pub fn merge_outside_characters(
    region_data: std::collections::HashMap<ServerRegion, Vec<OutsidecharacterElement>>,
) -> Vec<UniversalOutsideCharacter> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalOutsideCharacter> = by_id
        .values()
        .filter_map(UniversalOutsideCharacter::from_regional)
        .collect();
    result.sort_by_key(|o| o.id);
    result
}

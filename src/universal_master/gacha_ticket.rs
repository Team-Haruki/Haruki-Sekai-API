//! UniversalGachaTicket - Mixed: name is regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::gachatickets::GachaticketElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for GachaticketElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalGachaTicket {
    pub id: i64,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_display_type: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalGachaTicket {
    pub fn from_regional(
        regional: &super::types::RegionalData<GachaticketElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |g| g.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalGachaTicket {
            id,
            name: merge_field(regional, |g| g.name.clone()),
            assetbundle_name: get_first_value(regional, |g| g.assetbundle_name.clone()),
            gacha_display_type: get_first_value(regional, |g| {
                g.gacha_display_type.as_ref().map(|t| format!("{:?}", t))
            }),
            available_regions,
        })
    }
}

pub fn merge_gacha_tickets(
    region_data: std::collections::HashMap<ServerRegion, Vec<GachaticketElement>>,
) -> Vec<UniversalGachaTicket> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalGachaTicket> = by_id
        .values()
        .filter_map(UniversalGachaTicket::from_regional)
        .collect();
    result.sort_by_key(|g| g.id);
    result
}

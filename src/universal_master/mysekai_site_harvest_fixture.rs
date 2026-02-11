//! UniversalMysekaiSiteHarvestFixture - Merged harvest fixture data
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaisiteharvestfixtures::MysekaisiteharvestfixtureElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaisiteharvestfixtureElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiSiteHarvestFixture {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_site_harvest_fixture_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hp: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attack_stamina: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_site_harvest_fixture_rarity_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiSiteHarvestFixture {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaisiteharvestfixtureElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiSiteHarvestFixture {
            id,
            mysekai_site_harvest_fixture_type: get_first_value(regional, |m| {
                m.mysekai_site_harvest_fixture_type.clone()
            }),
            hp: get_first_value(regional, |m| m.hp),
            last_attack_stamina: get_first_value(regional, |m| m.last_attack_stamina),
            mysekai_site_harvest_fixture_rarity_type: get_first_value(regional, |m| {
                m.mysekai_site_harvest_fixture_rarity_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            assetbundle_name: get_first_value(regional, |m| m.assetbundle_name.clone()),
            available_regions,
        })
    }
}

pub fn merge_mysekai_site_harvest_fixtures(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaisiteharvestfixtureElement>>,
) -> Vec<UniversalMysekaiSiteHarvestFixture> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiSiteHarvestFixture> = by_id
        .values()
        .filter_map(UniversalMysekaiSiteHarvestFixture::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

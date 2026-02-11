//! UniversalMysekaiFixtureOnlyDisassembleMaterial
//! Fully universal — no regional differences.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaifixtureonlydisassemblematerials::MysekaifixtureonlydisassemblematerialElement;

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MysekaifixtureonlydisassemblematerialElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiFixtureOnlyDisassembleMaterial {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_fixture_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_material_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiFixtureOnlyDisassembleMaterial {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaifixtureonlydisassemblematerialElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiFixtureOnlyDisassembleMaterial {
            id,
            mysekai_fixture_id: get_first_value(regional, |m| m.mysekai_fixture_id),
            mysekai_material_id: get_first_value(regional, |m| m.mysekai_material_id),
            seq: get_first_value(regional, |m| m.seq),
            quantity: get_first_value(regional, |m| m.quantity),
            available_regions,
        })
    }
}

pub fn merge_mysekai_fixture_only_disassemble_materials(
    region_data: std::collections::HashMap<
        ServerRegion,
        Vec<MysekaifixtureonlydisassemblematerialElement>,
    >,
) -> Vec<UniversalMysekaiFixtureOnlyDisassembleMaterial> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiFixtureOnlyDisassembleMaterial> = by_id
        .values()
        .filter_map(UniversalMysekaiFixtureOnlyDisassembleMaterial::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

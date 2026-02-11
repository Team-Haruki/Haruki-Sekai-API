//! UniversalMysekaiFixtureSubGenre - Mixed: name is regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaifixturesubgenres::MysekaifixturesubgenreElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for MysekaifixturesubgenreElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiFixtureSubGenre {
    pub id: i64,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_fixture_sub_genre_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiFixtureSubGenre {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaifixturesubgenreElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiFixtureSubGenre {
            id,
            name: merge_field(regional, |m| m.name.clone()),
            mysekai_fixture_sub_genre_type: get_first_value(regional, |m| {
                m.mysekai_fixture_sub_genre_type.clone()
            }),
            assetbundle_name: get_first_value(regional, |m| m.assetbundle_name.clone()),
            available_regions,
        })
    }
}

pub fn merge_mysekai_fixture_sub_genres(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaifixturesubgenreElement>>,
) -> Vec<UniversalMysekaiFixtureSubGenre> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiFixtureSubGenre> = by_id
        .values()
        .filter_map(UniversalMysekaiFixtureSubGenre::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

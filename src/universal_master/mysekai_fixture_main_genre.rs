//! UniversalMysekaiFixtureMainGenre - Mixed: name is regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaifixturemaingenres::MysekaifixturemaingenreElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for MysekaifixturemaingenreElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiFixtureMainGenre {
    pub id: i64,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_fixture_main_genre_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiFixtureMainGenre {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaifixturemaingenreElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiFixtureMainGenre {
            id,
            name: merge_field(regional, |m| m.name.clone()),
            mysekai_fixture_main_genre_type: get_first_value(regional, |m| {
                m.mysekai_fixture_main_genre_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            assetbundle_name: get_first_value(regional, |m| m.assetbundle_name.clone()),
            group_id: get_first_value(regional, |m| m.group_id.flatten()),
            available_regions,
        })
    }
}

pub fn merge_mysekai_fixture_main_genres(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaifixturemaingenreElement>>,
) -> Vec<UniversalMysekaiFixtureMainGenre> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiFixtureMainGenre> = by_id
        .values()
        .filter_map(UniversalMysekaiFixtureMainGenre::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

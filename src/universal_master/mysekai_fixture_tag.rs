//! UniversalMysekaiFixtureTag - Mixed: name, pronunciation are regional
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::mysekaifixturetags::MysekaifixturetagElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for MysekaifixturetagElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMysekaiFixtureTag {
    pub id: i64,

    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronunciation: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mysekai_fixture_tag_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMysekaiFixtureTag {
    pub fn from_regional(
        regional: &super::types::RegionalData<MysekaifixturetagElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |m| m.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalMysekaiFixtureTag {
            id,
            name: merge_field(regional, |m| m.name.clone()),
            pronunciation: {
                let field = merge_field(regional, |m| m.pronunciation.clone());
                Some(field)
            },
            mysekai_fixture_tag_type: get_first_value(regional, |m| {
                m.mysekai_fixture_tag_type
                    .as_ref()
                    .map(|t| format!("{:?}", t))
            }),
            external_id: get_first_value(regional, |m| m.external_id.flatten()),
            available_regions,
        })
    }
}

pub fn merge_mysekai_fixture_tags(
    region_data: std::collections::HashMap<ServerRegion, Vec<MysekaifixturetagElement>>,
) -> Vec<UniversalMysekaiFixtureTag> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMysekaiFixtureTag> = by_id
        .values()
        .filter_map(UniversalMysekaiFixtureTag::from_regional)
        .collect();
    result.sort_by_key(|m| m.id);
    result
}

//! UniversalEventItem - Mixed: name, flavor_text are regional
//! Both `Name` and `FlavorText` are enums with localized text — serialized via Debug
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventitems::EventitemElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for EventitemElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventItem {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,

    /// Name enum contains localized strings — serialized via Debug
    pub name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub flavor_text: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventItem {
    pub fn from_regional(regional: &super::types::RegionalData<EventitemElement>) -> Option<Self> {
        let id = get_first_value(regional, |e| e.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalEventItem {
            id,
            event_id: get_first_value(regional, |e| e.event_id),
            name: merge_field(regional, |e| e.name.as_ref().map(|n| format!("{:?}", n))),
            flavor_text: Some(merge_field(regional, |e| {
                e.flavor_text
                    .as_ref()
                    .and_then(|ft| ft.as_ref().map(|f| format!("{:?}", f)))
            })),
            assetbundle_name: get_first_value(regional, |e| {
                e.assetbundle_name.clone().and_then(|v| v)
            }),
            game_character_id: get_first_value(regional, |e| e.game_character_id.flatten()),
            available_regions,
        })
    }
}

pub fn merge_event_items(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventitemElement>>,
) -> Vec<UniversalEventItem> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventItem> = by_id
        .values()
        .filter_map(UniversalEventItem::from_regional)
        .collect();
    result.sort_by_key(|e| e.id);
    result
}

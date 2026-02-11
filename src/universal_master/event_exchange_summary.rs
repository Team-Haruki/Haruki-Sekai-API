//! UniversalEventExchangeSummary - Mixed: start_at, end_at are regional
//! Nested event_exchanges taken from JP (structure identical across regions)
use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::eventexchangesummaries::EventexchangesummarieElement;

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for EventexchangesummarieElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

/// Flattened event exchange cost
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventExchangeCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_quantity: Option<i64>,
}

/// Flattened event exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventExchange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_box_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange_limit: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_exchange_cost: Option<UniversalEventExchangeCost>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_character_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEventExchangeSummary {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_at: Option<UnifiedValue<i64>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_at: Option<UnifiedValue<i64>>,

    /// Nested exchanges taken from JP (identical structure across regions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_exchanges: Option<Vec<UniversalEventExchange>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEventExchangeSummary {
    pub fn from_regional(
        regional: &super::types::RegionalData<EventexchangesummarieElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |e| e.id)?;
        let available_regions = regional.available_regions();

        Some(UniversalEventExchangeSummary {
            id,
            event_id: get_first_value(regional, |e| e.event_id),
            assetbundle_name: get_first_value(regional, |e| e.assetbundle_name.clone()),
            start_at: Some(merge_field(regional, |e| e.start_at)),
            end_at: Some(merge_field(regional, |e| e.end_at)),
            event_exchanges: get_first_value(regional, |e| {
                e.event_exchanges.as_ref().map(|exchanges| {
                    exchanges
                        .iter()
                        .map(|ex| UniversalEventExchange {
                            id: ex.id,
                            seq: ex.seq,
                            resource_box_id: ex.resource_box_id,
                            exchange_limit: ex.exchange_limit.and_then(|v| v),
                            event_exchange_cost: ex.event_exchange_cost.as_ref().map(|c| {
                                UniversalEventExchangeCost {
                                    resource_type: c
                                        .resource_type
                                        .as_ref()
                                        .and_then(|v| v.as_ref().map(|t| format!("{:?}", t))),
                                    resource_id: c.resource_id.and_then(|v| v),
                                    resource_quantity: c.resource_quantity,
                                }
                            }),
                            game_character_id: ex.game_character_id.and_then(|v| v),
                        })
                        .collect()
                })
            }),
            available_regions,
        })
    }
}

pub fn merge_event_exchange_summaries(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventexchangesummarieElement>>,
) -> Vec<UniversalEventExchangeSummary> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEventExchangeSummary> = by_id
        .values()
        .filter_map(UniversalEventExchangeSummary::from_regional)
        .collect();
    result.sort_by_key(|e| e.id);
    result
}

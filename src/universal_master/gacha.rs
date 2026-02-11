//! UniversalGacha - Merged gacha banner data across all regions
//! Most complex mixed model: rates and details are Regional.

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::gachas::{
    GachaBehavior, GachaCardRarityRate, GachaDetail, GachaElement, GachaPickup,
};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for GachaElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalGacha {
    pub id: i64,

    // Universal fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_card_rarity_rate_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_ceil_item_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub wish_select_count: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub wish_fixed_select_count: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub wish_limited_select_count: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_behaviors: Option<Vec<GachaBehavior>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_pickups: Option<Vec<GachaPickup>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_pickup_costumes: Option<Vec<serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_bonus_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub spin_limit: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_spin_limit: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_bonus_item_receivable_reward_group_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_freebie_group_id: Option<i64>,

    // Regional fields
    pub name: UnifiedValue<String>,

    pub start_at: UnifiedValue<i64>,

    pub end_at: UnifiedValue<i64>,

    /// Gacha rates may differ per region (especially CN due to regulations)
    pub gacha_card_rarity_rates: UnifiedValue<Vec<GachaCardRarityRate>>,

    /// Gacha card pools may differ per region
    pub gacha_details: UnifiedValue<Vec<GachaDetail>>,

    /// Localized gacha information (summary + description flattened)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_information_summary: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_information_description: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_show_period: Option<UnifiedValue<bool>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub drawable_gacha_hour: Option<UnifiedValue<i64>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalGacha {
    pub fn from_regional(regional: &super::types::RegionalData<GachaElement>) -> Option<Self> {
        let id = get_first_value(regional, |g| g.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let gacha_type = get_first_value(regional, |g| {
            g.gacha_type.as_ref().map(|t| format!("{:?}", t))
        });
        let assetbundle_name = get_first_value(regional, |g| g.assetbundle_name.clone());
        let gacha_card_rarity_rate_group_id =
            get_first_value(regional, |g| g.gacha_card_rarity_rate_group_id.flatten());
        let gacha_ceil_item_id = get_first_value(regional, |g| g.gacha_ceil_item_id.flatten());
        let wish_select_count = get_first_value(regional, |g| g.wish_select_count);
        let wish_fixed_select_count = get_first_value(regional, |g| g.wish_fixed_select_count);
        let wish_limited_select_count = get_first_value(regional, |g| g.wish_limited_select_count);
        let gacha_behaviors = get_first_value(regional, |g| g.gacha_behaviors.clone());
        let gacha_pickups = get_first_value(regional, |g| g.gacha_pickups.clone());
        let gacha_pickup_costumes = get_first_value(regional, |g| {
            g.gacha_pickup_costumes
                .clone()
                .flatten()
                .map(|v| v.into_iter().filter_map(|x| x).collect::<Vec<_>>())
        });
        let gacha_bonus_id = get_first_value(regional, |g| g.gacha_bonus_id.flatten());
        let spin_limit = get_first_value(regional, |g| g.spin_limit.flatten());
        let daily_spin_limit = get_first_value(regional, |g| g.daily_spin_limit.flatten());
        let gacha_bonus_item_receivable_reward_group_id = get_first_value(regional, |g| {
            g.gacha_bonus_item_receivable_reward_group_id.flatten()
        });
        let gacha_freebie_group_id =
            get_first_value(regional, |g| g.gacha_freebie_group_id.flatten());

        // Regional fields
        let name = merge_field(regional, |g| g.name.clone());
        let start_at = merge_field(regional, |g| g.start_at);
        let end_at = merge_field(regional, |g| g.end_at);

        let gacha_card_rarity_rates = merge_field(regional, |g| g.gacha_card_rarity_rates.clone());
        let gacha_details = merge_field(regional, |g| g.gacha_details.clone());

        // Flatten gacha_information into separate summary/description fields
        let gacha_information_summary = {
            let v = merge_field(regional, |g| {
                g.gacha_information.as_ref().and_then(|i| i.summary.clone())
            });
            Some(v)
        };
        let gacha_information_description = {
            let v = merge_field(regional, |g| {
                g.gacha_information
                    .as_ref()
                    .and_then(|i| i.description.clone())
            });
            Some(v)
        };
        let is_show_period = {
            let v = merge_field(regional, |g| g.is_show_period);
            Some(v)
        };
        let drawable_gacha_hour = {
            let v = merge_field(regional, |g| g.drawable_gacha_hour.flatten());
            Some(v)
        };

        Some(UniversalGacha {
            id,
            gacha_type,
            assetbundle_name,
            gacha_card_rarity_rate_group_id,
            gacha_ceil_item_id,
            wish_select_count,
            wish_fixed_select_count,
            wish_limited_select_count,
            gacha_behaviors,
            gacha_pickups,
            gacha_pickup_costumes,
            gacha_bonus_id,
            spin_limit,
            daily_spin_limit,
            gacha_bonus_item_receivable_reward_group_id,
            gacha_freebie_group_id,
            name,
            start_at,
            end_at,
            gacha_card_rarity_rates,
            gacha_details,
            gacha_information_summary,
            gacha_information_description,
            is_show_period,
            drawable_gacha_hour,
            available_regions,
        })
    }
}

pub fn merge_gachas(
    region_data: std::collections::HashMap<ServerRegion, Vec<GachaElement>>,
) -> Vec<UniversalGacha> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalGacha> = by_id
        .values()
        .filter_map(UniversalGacha::from_regional)
        .collect();
    result.sort_by_key(|g| g.id);
    result
}

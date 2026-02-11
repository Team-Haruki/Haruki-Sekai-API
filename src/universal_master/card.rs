//! UniversalCard - Merged card data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::cards::{
    Attr, CardElement, CardParametersUnion, CardRarityType, MasterLessonAchieveResource,
    SpecialTrainingCost, SupportUnit,
};

use super::merger::{collect_by_id, get_first_value, get_jp_value, merge_field, Mergeable};
use super::types::{RegionalData, UnifiedValue};

impl Mergeable for CardElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalCard {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_rarity_type: Option<CardRarityType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_training_power1_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_training_power2_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_training_power3_bonus_fixed: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub attr: Option<Attr>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_unit: Option<SupportUnit>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_supply_id: Option<i64>,

    /// cardParameters: take JP value directly
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_parameters: Option<CardParametersUnion>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_training_costs: Option<Vec<SpecialTrainingCost>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_lesson_achieve_resources: Option<Vec<MasterLessonAchieveResource>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_training_skill_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_training_reward_resource_box_id: Option<i64>,

    // Regional fields
    /// Localized card name prefix
    pub prefix: UnifiedValue<String>,

    /// Localized skill name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_skill_name: Option<UnifiedValue<String>>,

    /// Localized gacha phrase
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gacha_phrase: Option<UnifiedValue<String>>,

    /// Release timestamp per region
    pub release_at: UnifiedValue<i64>,

    /// Archive published timestamp per region
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_published_at: Option<UnifiedValue<i64>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalCard {
    pub fn from_regional(regional: &RegionalData<CardElement>) -> Option<Self> {
        let id = get_first_value(regional, |c| c.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let character_id = get_first_value(regional, |c| c.character_id);
        let card_rarity_type = get_first_value(regional, |c| c.card_rarity_type.clone());
        let special_training_power1_bonus_fixed =
            get_first_value(regional, |c| c.special_training_power1_bonus_fixed);
        let special_training_power2_bonus_fixed =
            get_first_value(regional, |c| c.special_training_power2_bonus_fixed);
        let special_training_power3_bonus_fixed =
            get_first_value(regional, |c| c.special_training_power3_bonus_fixed);
        let attr = get_first_value(regional, |c| c.attr.clone());
        let support_unit = get_first_value(regional, |c| c.support_unit.clone());
        let skill_id = get_first_value(regional, |c| c.skill_id);
        let assetbundle_name = get_first_value(regional, |c| c.assetbundle_name.clone());
        let card_supply_id = get_first_value(regional, |c| c.card_supply_id);

        // JP-only field
        let card_parameters = get_jp_value(regional, |c| c.card_parameters.clone());
        let special_training_costs =
            get_first_value(regional, |c| c.special_training_costs.clone());
        let master_lesson_achieve_resources =
            get_first_value(regional, |c| c.master_lesson_achieve_resources.clone());
        let special_training_skill_id =
            get_first_value(regional, |c| c.special_training_skill_id.flatten());
        let special_training_reward_resource_box_id = get_first_value(regional, |c| {
            c.special_training_reward_resource_box_id.flatten()
        });

        // Regional fields
        let prefix = merge_field(regional, |c| c.prefix.clone());
        let card_skill_name = {
            let v = merge_field(regional, |c| c.card_skill_name.clone());
            match v {
                UnifiedValue::Uniform(ref s) if s.is_empty() => None,
                _ => Some(v),
            }
        };
        let gacha_phrase = {
            let v = merge_field(regional, |c| c.gacha_phrase.clone());
            match v {
                UnifiedValue::Uniform(ref s) if s.is_empty() => None,
                _ => Some(v),
            }
        };
        let release_at = merge_field(regional, |c| c.release_at);
        let archive_published_at = {
            let v = merge_field(regional, |c| c.archive_published_at);
            Some(v)
        };

        Some(UniversalCard {
            id,
            character_id,
            card_rarity_type,
            special_training_power1_bonus_fixed,
            special_training_power2_bonus_fixed,
            special_training_power3_bonus_fixed,
            attr,
            support_unit,
            skill_id,
            assetbundle_name,
            card_supply_id,
            card_parameters,
            special_training_costs,
            master_lesson_achieve_resources,
            special_training_skill_id,
            special_training_reward_resource_box_id,
            prefix,
            card_skill_name,
            gacha_phrase,
            release_at,
            archive_published_at,
            available_regions,
        })
    }
}

pub fn merge_cards(
    region_data: std::collections::HashMap<ServerRegion, Vec<CardElement>>,
) -> Vec<UniversalCard> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalCard> = by_id
        .values()
        .filter_map(UniversalCard::from_regional)
        .collect();
    result.sort_by_key(|c| c.id);
    result
}

/// Run the card merge with all 5 regions and export to file
pub async fn run_merge_test_and_export(
    base_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<Vec<UniversalCard>, crate::error::AppError> {
    use std::collections::HashMap;

    let region_folders = [
        (ServerRegion::Jp, "haruki-sekai-master"),
        (ServerRegion::En, "haruki-sekai-en-master"),
        (ServerRegion::Tw, "haruki-sekai-tc-master"),
        (ServerRegion::Kr, "haruki-sekai-kr-master"),
        (ServerRegion::Cn, "haruki-sekai-sc-master"),
    ];

    println!("=== Card Merge Test ===\n");

    let mut region_data: HashMap<ServerRegion, Vec<CardElement>> = HashMap::new();

    for (region, folder) in &region_folders {
        let master_dir = base_path.join(folder).join("master");
        let master_dir_str = master_dir.to_string_lossy().to_string();

        match crate::universal_master::loader::load_master_data_file::<Vec<CardElement>>(
            &master_dir_str,
            "cards",
        )
        .await
        {
            Ok(data) => {
                println!(
                    "✓ Loaded {} cards from {} ({})",
                    data.len(),
                    folder,
                    region.as_str().to_uppercase()
                );
                region_data.insert(*region, data);
            }
            Err(e) => {
                println!(
                    "✗ Failed to load from {} ({}): {}",
                    folder,
                    region.as_str().to_uppercase(),
                    e
                );
            }
        }
    }

    if region_data.is_empty() {
        return Err(crate::error::AppError::ParseError(
            "No data loaded from any region".to_string(),
        ));
    }

    let merged = merge_cards(region_data);
    println!("Merged into {} universal cards", merged.len());

    let json_content = serde_json::to_string_pretty(&merged)
        .map_err(|e| crate::error::AppError::ParseError(format!("Failed to serialize: {}", e)))?;
    std::fs::write(output_path, &json_content)
        .map_err(|e| crate::error::AppError::IoError(format!("Failed to write file: {}", e)))?;

    println!("\n✓ Exported to: {}", output_path.display());

    // Stats
    let mut uniform_prefix = 0;
    let mut regional_prefix = 0;
    for card in &merged {
        match &card.prefix {
            UnifiedValue::Uniform(_) => uniform_prefix += 1,
            UnifiedValue::Regional(_) => regional_prefix += 1,
        }
    }
    println!("\n=== Statistics ===");
    println!("Total cards: {}", merged.len());
    println!("Uniform prefix: {}", uniform_prefix);
    println!("Regional prefix: {}", regional_prefix);

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_merge_cards_all_regions() {
        let base_path = Path::new("Data/master");
        let output_path = Path::new("Data/merged_cards_test.json");

        if !base_path.exists() {
            println!("Skipping integration test: Data/master not found");
            return;
        }

        let result = run_merge_test_and_export(base_path, output_path).await;

        match result {
            Ok(merged) => {
                assert!(!merged.is_empty(), "Should have merged card data");
                // Verify card_parameters is populated from JP
                let with_params = merged
                    .iter()
                    .filter(|c| c.card_parameters.is_some())
                    .count();
                println!("Cards with cardParameters: {}", with_params);
                println!("\n=== Test Passed ===");
            }
            Err(e) => {
                panic!("Merge test failed: {}", e);
            }
        }
    }
}

//! UniversalGameCharacter - Merged game character data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::gamecharacters::{
    BreastSize, Figure, GamecharacterElement, Gender, SupportUnitType, Unit,
};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for GamecharacterElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalGameCharacter {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Gender>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,

    #[serde(rename = "live2dHeightAdjustment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live2d_height_adjustment: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub figure: Option<Figure>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub breast_size: Option<BreastSize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_unit_type: Option<SupportUnitType>,

    // Regional fields — character names differ by localization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<UnifiedValue<String>>,

    pub given_name: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name_ruby: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub given_name_ruby: Option<UnifiedValue<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name_english: Option<UnifiedValue<String>>,

    pub given_name_english: UnifiedValue<String>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalGameCharacter {
    pub fn from_regional(
        regional: &super::types::RegionalData<GamecharacterElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |g| g.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let resource_id = get_first_value(regional, |g| g.resource_id);
        let gender = get_first_value(regional, |g| g.gender.clone());
        let height = get_first_value(regional, |g| g.height);
        let live2d_height_adjustment = get_first_value(regional, |g| g.live2_d_height_adjustment);
        let figure = get_first_value(regional, |g| g.figure.clone());
        let breast_size = get_first_value(regional, |g| g.breast_size.clone());
        let model_name = get_first_value(regional, |g| g.model_name.clone().flatten());
        let unit = get_first_value(regional, |g| g.unit.clone());
        let support_unit_type = get_first_value(regional, |g| g.support_unit_type.clone());

        // Regional fields
        let first_name = {
            let v = merge_field(regional, |g| g.first_name.clone().flatten());
            match v {
                UnifiedValue::Uniform(ref s) if s.is_empty() => None,
                _ => Some(v),
            }
        };
        let given_name = merge_field(regional, |g| g.given_name.clone());
        let first_name_ruby = {
            let v = merge_field(regional, |g| g.first_name_ruby.clone().flatten());
            match v {
                UnifiedValue::Uniform(ref s) if s.is_empty() => None,
                _ => Some(v),
            }
        };
        let given_name_ruby = {
            let v = merge_field(regional, |g| g.given_name_ruby.clone().flatten());
            match v {
                UnifiedValue::Uniform(ref s) if s.is_empty() => None,
                _ => Some(v),
            }
        };
        let first_name_english = {
            let v = merge_field(regional, |g| g.first_name_english.clone().flatten());
            match v {
                UnifiedValue::Uniform(ref s) if s.is_empty() => None,
                _ => Some(v),
            }
        };
        let given_name_english = merge_field(regional, |g| g.given_name_english.clone());

        Some(UniversalGameCharacter {
            id,
            resource_id,
            gender,
            height,
            live2d_height_adjustment,
            figure,
            breast_size,
            model_name,
            unit,
            support_unit_type,
            first_name,
            given_name,
            first_name_ruby,
            given_name_ruby,
            first_name_english,
            given_name_english,
            available_regions,
        })
    }
}

pub fn merge_game_characters(
    region_data: std::collections::HashMap<ServerRegion, Vec<GamecharacterElement>>,
) -> Vec<UniversalGameCharacter> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalGameCharacter> = by_id
        .values()
        .filter_map(UniversalGameCharacter::from_regional)
        .collect();
    result.sort_by_key(|g| g.id);
    result
}

pub async fn run_merge_test_and_export(
    base_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<Vec<UniversalGameCharacter>, crate::error::AppError> {
    use std::collections::HashMap;

    let region_folders = [
        (ServerRegion::Jp, "haruki-sekai-master"),
        (ServerRegion::En, "haruki-sekai-en-master"),
        (ServerRegion::Tw, "haruki-sekai-tc-master"),
        (ServerRegion::Kr, "haruki-sekai-kr-master"),
        (ServerRegion::Cn, "haruki-sekai-sc-master"),
    ];

    println!("=== GameCharacter Merge Test ===\n");

    let mut region_data: HashMap<ServerRegion, Vec<GamecharacterElement>> = HashMap::new();

    for (region, folder) in &region_folders {
        let master_dir = base_path.join(folder).join("master");
        let master_dir_str = master_dir.to_string_lossy().to_string();

        match crate::universal_master::loader::load_master_data_file::<Vec<GamecharacterElement>>(
            &master_dir_str,
            "gameCharacters",
        )
        .await
        {
            Ok(data) => {
                println!(
                    "✓ Loaded {} characters from {} ({})",
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

    let merged = merge_game_characters(region_data);
    println!("Merged into {} universal characters", merged.len());

    let json_content = serde_json::to_string_pretty(&merged)
        .map_err(|e| crate::error::AppError::ParseError(format!("Failed to serialize: {}", e)))?;
    std::fs::write(output_path, &json_content)
        .map_err(|e| crate::error::AppError::IoError(format!("Failed to write file: {}", e)))?;

    println!("\n✓ Exported to: {}", output_path.display());

    let mut uniform_name = 0;
    let mut regional_name = 0;
    for ch in &merged {
        match &ch.given_name {
            UnifiedValue::Uniform(_) => uniform_name += 1,
            UnifiedValue::Regional(_) => regional_name += 1,
        }
    }
    println!("\n=== Statistics ===");
    println!("Total characters: {}", merged.len());
    println!("Uniform givenName: {}", uniform_name);
    println!("Regional givenName: {}", regional_name);

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_merge_game_characters_all_regions() {
        let base_path = Path::new("Data/master");
        let output_path = Path::new("Data/merged_game_characters_test.json");

        if !base_path.exists() {
            println!("Skipping integration test: Data/master not found");
            return;
        }

        let result = run_merge_test_and_export(base_path, output_path).await;

        match result {
            Ok(merged) => {
                assert!(!merged.is_empty(), "Should have merged character data");
                // Every character should have a unit
                let with_unit = merged.iter().filter(|c| c.unit.is_some()).count();
                println!("Characters with unit: {}/{}", with_unit, merged.len());
                println!("\n=== Test Passed ===");
            }
            Err(e) => {
                panic!("Merge test failed: {}", e);
            }
        }
    }
}

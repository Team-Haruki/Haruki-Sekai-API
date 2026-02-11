//! UniversalSkill - Merged skill data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::skills::{DescriptionSpriteName, SkillEffect, SkillElement};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for SkillElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalSkill {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_sprite_name: Option<DescriptionSpriteName>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_filter_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_effects: Option<Vec<SkillEffect>>,

    // Regional fields
    /// Localized skill description
    pub description: UnifiedValue<String>,

    /// Localized short description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_description: Option<UnifiedValue<String>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalSkill {
    pub fn from_regional(regional: &super::types::RegionalData<SkillElement>) -> Option<Self> {
        let id = get_first_value(regional, |s| s.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let description_sprite_name =
            get_first_value(regional, |s| s.description_sprite_name.clone());
        let skill_filter_id = get_first_value(regional, |s| s.skill_filter_id);
        let skill_effects = get_first_value(regional, |s| s.skill_effects.clone());

        // Regional fields
        let description = merge_field(regional, |s| s.description.clone());
        let short_description = {
            let v = merge_field(regional, |s| s.short_description.clone().flatten());
            match v {
                UnifiedValue::Uniform(ref s) if s.is_empty() => None,
                _ => Some(v),
            }
        };

        Some(UniversalSkill {
            id,
            description_sprite_name,
            skill_filter_id,
            skill_effects,
            description,
            short_description,
            available_regions,
        })
    }
}

pub fn merge_skills(
    region_data: std::collections::HashMap<ServerRegion, Vec<SkillElement>>,
) -> Vec<UniversalSkill> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalSkill> = by_id
        .values()
        .filter_map(UniversalSkill::from_regional)
        .collect();
    result.sort_by_key(|s| s.id);
    result
}

pub async fn run_merge_test_and_export(
    base_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<Vec<UniversalSkill>, crate::error::AppError> {
    use std::collections::HashMap;

    let region_folders = [
        (ServerRegion::Jp, "haruki-sekai-master"),
        (ServerRegion::En, "haruki-sekai-en-master"),
        (ServerRegion::Tw, "haruki-sekai-tc-master"),
        (ServerRegion::Kr, "haruki-sekai-kr-master"),
        (ServerRegion::Cn, "haruki-sekai-sc-master"),
    ];

    println!("=== Skill Merge Test ===\n");

    let mut region_data: HashMap<ServerRegion, Vec<SkillElement>> = HashMap::new();

    for (region, folder) in &region_folders {
        let master_dir = base_path.join(folder).join("master");
        let master_dir_str = master_dir.to_string_lossy().to_string();

        match crate::universal_master::loader::load_master_data_file::<Vec<SkillElement>>(
            &master_dir_str,
            "skills",
        )
        .await
        {
            Ok(data) => {
                println!(
                    "✓ Loaded {} skills from {} ({})",
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

    let merged = merge_skills(region_data);
    println!("Merged into {} universal skills", merged.len());

    let json_content = serde_json::to_string_pretty(&merged)
        .map_err(|e| crate::error::AppError::ParseError(format!("Failed to serialize: {}", e)))?;
    std::fs::write(output_path, &json_content)
        .map_err(|e| crate::error::AppError::IoError(format!("Failed to write file: {}", e)))?;

    println!("\n✓ Exported to: {}", output_path.display());

    let mut uniform_desc = 0;
    let mut regional_desc = 0;
    for skill in &merged {
        match &skill.description {
            UnifiedValue::Uniform(_) => uniform_desc += 1,
            UnifiedValue::Regional(_) => regional_desc += 1,
        }
    }
    println!("\n=== Statistics ===");
    println!("Total skills: {}", merged.len());
    println!("Uniform descriptions: {}", uniform_desc);
    println!("Regional descriptions: {}", regional_desc);

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_merge_skills_all_regions() {
        let base_path = Path::new("Data/master");
        let output_path = Path::new("Data/merged_skills_test.json");

        if !base_path.exists() {
            println!("Skipping integration test: Data/master not found");
            return;
        }

        let result = run_merge_test_and_export(base_path, output_path).await;

        match result {
            Ok(merged) => {
                assert!(!merged.is_empty(), "Should have merged skill data");
                println!("\n=== Test Passed ===");
            }
            Err(e) => {
                panic!("Merge test failed: {}", e);
            }
        }
    }
}

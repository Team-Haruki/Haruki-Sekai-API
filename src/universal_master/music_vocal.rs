//! UniversalMusicVocal - Merged music vocal data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::musicvocals::{Character, MusicVocalType, MusicvocalElement};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for MusicvocalElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMusicVocal {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_vocal_type: Option<MusicVocalType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub characters: Option<Vec<Character>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_season_id: Option<i64>,

    // Regional fields
    /// Caption differs per region localization (e.g. "セカイver." vs "世界ver.")
    pub caption: UnifiedValue<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_published_at: Option<UnifiedValue<i64>>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMusicVocal {
    pub fn from_regional(regional: &super::types::RegionalData<MusicvocalElement>) -> Option<Self> {
        let id = get_first_value(regional, |v| v.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let music_id = get_first_value(regional, |v| v.music_id);
        let music_vocal_type = get_first_value(regional, |v| v.music_vocal_type.clone());
        let characters = get_first_value(regional, |v| v.characters.clone());
        let assetbundle_name = get_first_value(regional, |v| v.assetbundle_name.clone());
        let special_season_id = get_first_value(regional, |v| v.special_season_id.flatten());

        // Regional fields
        let caption = merge_field(regional, |v| v.caption.clone());
        let archive_published_at = {
            let v = merge_field(regional, |v| v.archive_published_at.flatten());
            Some(v)
        };

        Some(UniversalMusicVocal {
            id,
            music_id,
            music_vocal_type,
            characters,
            assetbundle_name,
            special_season_id,
            caption,
            archive_published_at,
            available_regions,
        })
    }
}

pub fn merge_music_vocals(
    region_data: std::collections::HashMap<ServerRegion, Vec<MusicvocalElement>>,
) -> Vec<UniversalMusicVocal> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMusicVocal> = by_id
        .values()
        .filter_map(UniversalMusicVocal::from_regional)
        .collect();
    result.sort_by_key(|v| v.id);
    result
}

pub async fn run_merge_test_and_export(
    base_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<Vec<UniversalMusicVocal>, crate::error::AppError> {
    use std::collections::HashMap;

    let region_folders = [
        (ServerRegion::Jp, "haruki-sekai-master"),
        (ServerRegion::En, "haruki-sekai-en-master"),
        (ServerRegion::Tw, "haruki-sekai-tc-master"),
        (ServerRegion::Kr, "haruki-sekai-kr-master"),
        (ServerRegion::Cn, "haruki-sekai-sc-master"),
    ];

    println!("=== MusicVocal Merge Test ===\n");

    let mut region_data: HashMap<ServerRegion, Vec<MusicvocalElement>> = HashMap::new();

    for (region, folder) in &region_folders {
        let master_dir = base_path.join(folder).join("master");
        let master_dir_str = master_dir.to_string_lossy().to_string();

        match crate::universal_master::loader::load_master_data_file::<Vec<MusicvocalElement>>(
            &master_dir_str,
            "musicVocals",
        )
        .await
        {
            Ok(data) => {
                println!(
                    "✓ Loaded {} vocals from {} ({})",
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

    let merged = merge_music_vocals(region_data);
    println!("Merged into {} universal music vocals", merged.len());

    let json_content = serde_json::to_string_pretty(&merged)
        .map_err(|e| crate::error::AppError::ParseError(format!("Failed to serialize: {}", e)))?;
    std::fs::write(output_path, &json_content)
        .map_err(|e| crate::error::AppError::IoError(format!("Failed to write file: {}", e)))?;

    println!("\n✓ Exported to: {}", output_path.display());

    let mut uniform_cap = 0;
    let mut regional_cap = 0;
    for v in &merged {
        match &v.caption {
            UnifiedValue::Uniform(_) => uniform_cap += 1,
            UnifiedValue::Regional(_) => regional_cap += 1,
        }
    }
    println!("\n=== Statistics ===");
    println!("Total vocals: {}", merged.len());
    println!("Uniform caption: {}", uniform_cap);
    println!("Regional caption: {}", regional_cap);

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_merge_music_vocals_all_regions() {
        let base_path = Path::new("Data/master");
        let output_path = Path::new("Data/merged_music_vocals_test.json");

        if !base_path.exists() {
            println!("Skipping integration test: Data/master not found");
            return;
        }

        let result = run_merge_test_and_export(base_path, output_path).await;

        match result {
            Ok(merged) => {
                assert!(!merged.is_empty(), "Should have merged vocal data");
                // Every vocal should have a music_id
                for v in &merged {
                    assert!(v.music_id.is_some(), "music_id should always be present");
                }
                println!("\n=== Test Passed ===");
            }
            Err(e) => {
                panic!("Merge test failed: {}", e);
            }
        }
    }
}

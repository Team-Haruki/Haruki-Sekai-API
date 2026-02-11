//! UniversalMusicDifficulty - Merged music difficulty data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::musicdifficulties::{MusicDifficulty, MusicdifficultieElement};

use super::merger::{collect_by_id, get_first_value, Mergeable};

impl Mergeable for MusicdifficultieElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

/// MusicDifficulty is almost entirely Universal — same chart data everywhere.
/// release_condition_id may differ but is rarely queried.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMusicDifficulty {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_difficulty: Option<MusicDifficulty>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_level: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_note_count: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMusicDifficulty {
    pub fn from_regional(
        regional: &super::types::RegionalData<MusicdifficultieElement>,
    ) -> Option<Self> {
        let id = get_first_value(regional, |d| d.id)?;
        let available_regions = regional.available_regions();

        let music_id = get_first_value(regional, |d| d.music_id);
        let music_difficulty = get_first_value(regional, |d| d.music_difficulty.clone());
        let play_level = get_first_value(regional, |d| d.play_level);
        let total_note_count = get_first_value(regional, |d| d.total_note_count);

        Some(UniversalMusicDifficulty {
            id,
            music_id,
            music_difficulty,
            play_level,
            total_note_count,
            available_regions,
        })
    }
}

pub fn merge_music_difficulties(
    region_data: std::collections::HashMap<ServerRegion, Vec<MusicdifficultieElement>>,
) -> Vec<UniversalMusicDifficulty> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalMusicDifficulty> = by_id
        .values()
        .filter_map(UniversalMusicDifficulty::from_regional)
        .collect();
    result.sort_by_key(|d| d.id);
    result
}

pub async fn run_merge_test_and_export(
    base_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<Vec<UniversalMusicDifficulty>, crate::error::AppError> {
    use std::collections::HashMap;

    let region_folders = [
        (ServerRegion::Jp, "haruki-sekai-master"),
        (ServerRegion::En, "haruki-sekai-en-master"),
        (ServerRegion::Tw, "haruki-sekai-tc-master"),
        (ServerRegion::Kr, "haruki-sekai-kr-master"),
        (ServerRegion::Cn, "haruki-sekai-sc-master"),
    ];

    println!("=== MusicDifficulty Merge Test ===\n");

    let mut region_data: HashMap<ServerRegion, Vec<MusicdifficultieElement>> = HashMap::new();

    for (region, folder) in &region_folders {
        let master_dir = base_path.join(folder).join("master");
        let master_dir_str = master_dir.to_string_lossy().to_string();

        match crate::universal_master::loader::load_master_data_file::<Vec<MusicdifficultieElement>>(
            &master_dir_str,
            "musicDifficulties",
        )
        .await
        {
            Ok(data) => {
                println!(
                    "✓ Loaded {} difficulties from {} ({})",
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

    let merged = merge_music_difficulties(region_data);
    println!("Merged into {} universal music difficulties", merged.len());

    let json_content = serde_json::to_string_pretty(&merged)
        .map_err(|e| crate::error::AppError::ParseError(format!("Failed to serialize: {}", e)))?;
    std::fs::write(output_path, &json_content)
        .map_err(|e| crate::error::AppError::IoError(format!("Failed to write file: {}", e)))?;

    println!("\n✓ Exported to: {}", output_path.display());
    println!("Total difficulties: {}", merged.len());

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_merge_music_difficulties_all_regions() {
        let base_path = Path::new("Data/master");
        let output_path = Path::new("Data/merged_music_difficulties_test.json");

        if !base_path.exists() {
            println!("Skipping integration test: Data/master not found");
            return;
        }

        let result = run_merge_test_and_export(base_path, output_path).await;

        match result {
            Ok(merged) => {
                assert!(!merged.is_empty(), "Should have merged difficulty data");
                // Every difficulty should have a music_id
                for d in &merged {
                    assert!(d.music_id.is_some(), "music_id should always be present");
                }
                println!("\n=== Test Passed ===");
            }
            Err(e) => {
                panic!("Merge test failed: {}", e);
            }
        }
    }
}

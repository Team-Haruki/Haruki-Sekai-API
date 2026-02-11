//! UniversalMusic - Merged music data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::musics::{CategoryElement, Info, MusicElement};

use super::merger::{
    collect_by_id, get_first_value, merge_field, merge_optional_regional_field, Mergeable,
};
use super::types::{RegionalData, UnifiedValue};

/// Implement Mergeable for MusicElement to enable cross-region merging
impl Mergeable for MusicElement {
    type Id = i64;

    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

/// Universal Music structure with merged region data
/// Uses the format specified in the requirements:
/// - Uniform fields: same across all regions, stored as scalar values
/// - Regional fields: differ per region, wrapped in regional object
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMusic {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<CategoryElement>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronunciation: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_artist_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyricist: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub composer: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub arranger: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub dancer_count: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_dancer_position: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_talk_background_assetbundle_name: Option<String>,

    /// Regional field: published_at timestamps differ per region
    pub published_at: UnifiedValue<i64>,

    /// Regional field: released_at timestamps differ per region
    pub released_at: UnifiedValue<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_stage_id: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub filler_sec: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_newly_written_music: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_full_length: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub music_collaboration_id: Option<i64>,

    /// Regional field: infos only present in TW/KR/CN servers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub infos: Option<RegionalData<Vec<Info>>>,

    /// List of regions where this music is available
    pub available_regions: Vec<ServerRegion>,
}

impl UniversalMusic {
    /// Merge music data from all regions into a UniversalMusic
    pub fn from_regional(regional: &RegionalData<MusicElement>) -> Option<Self> {
        // Get ID from first available region
        let id = get_first_value(regional, |m| m.id)?;

        // Build available_regions from present regions
        let available_regions = regional.available_regions();

        // Uniform fields - take from first available
        let categories = get_first_value(regional, |m| m.categories.clone());
        let title = get_first_value(regional, |m| m.title.clone());
        let pronunciation = get_first_value(regional, |m| m.pronunciation.clone());
        let creator_artist_id = get_first_value(regional, |m| m.creator_artist_id);
        let lyricist = get_first_value(regional, |m| m.lyricist.clone());
        let composer = get_first_value(regional, |m| m.composer.clone());
        let arranger = get_first_value(regional, |m| m.arranger.clone());
        let dancer_count = get_first_value(regional, |m| m.dancer_count);
        let self_dancer_position = get_first_value(regional, |m| m.self_dancer_position);
        let assetbundle_name = get_first_value(regional, |m| m.assetbundle_name.clone());
        let live_talk_background_assetbundle_name = get_first_value(regional, |m| {
            m.live_talk_background_assetbundle_name
                .clone()
                .flatten()
                .map(|v| format!("{:?}", v))
        });
        let live_stage_id = get_first_value(regional, |m| m.live_stage_id.clone().flatten());
        let filler_sec = get_first_value(regional, |m| m.filler_sec);
        let is_newly_written_music = get_first_value(regional, |m| m.is_newly_written_music);
        let is_full_length = get_first_value(regional, |m| m.is_full_length);
        let music_collaboration_id =
            get_first_value(regional, |m| m.music_collaboration_id.clone().flatten());

        // Regional fields - compare across regions
        let published_at = merge_field(regional, |m| m.published_at);
        let released_at = merge_field(regional, |m| m.released_at);

        // Optional regional field - only in TW/KR/CN
        let infos = merge_optional_regional_field(regional, |m| m.infos.clone().flatten());

        Some(UniversalMusic {
            id,
            categories,
            title,
            pronunciation,
            creator_artist_id,
            lyricist,
            composer,
            arranger,
            dancer_count,
            self_dancer_position,
            assetbundle_name,
            live_talk_background_assetbundle_name,
            published_at,
            released_at,
            live_stage_id,
            filler_sec,
            is_newly_written_music,
            is_full_length,
            music_collaboration_id,
            infos,
            available_regions,
        })
    }
}

/// Merge music data from multiple regions into a list of UniversalMusic
pub fn merge_musics(
    region_data: std::collections::HashMap<ServerRegion, Vec<MusicElement>>,
) -> Vec<UniversalMusic> {
    let by_id = collect_by_id(region_data);

    let mut result: Vec<UniversalMusic> = by_id
        .values()
        .filter_map(UniversalMusic::from_regional)
        .collect();

    // Sort by ID for consistent output
    result.sort_by_key(|m| m.id);

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_uniform_timestamps() {
        let mut jp_music = MusicElement {
            id: Some(1),
            published_at: Some(1000),
            released_at: Some(2000),
            ..Default::default()
        };

        let mut en_music = jp_music.clone();
        en_music.published_at = Some(1000); // Same as JP
        en_music.released_at = Some(2000); // Same as JP

        let mut region_data = std::collections::HashMap::new();
        region_data.insert(ServerRegion::Jp, vec![jp_music]);
        region_data.insert(ServerRegion::En, vec![en_music]);

        let merged = merge_musics(region_data);
        assert_eq!(merged.len(), 1);

        let music = &merged[0];
        assert!(matches!(music.published_at, UnifiedValue::Uniform(1000)));
        assert!(matches!(music.released_at, UnifiedValue::Uniform(2000)));
    }

    #[test]
    fn test_merge_regional_timestamps() {
        let jp_music = MusicElement {
            id: Some(1),
            published_at: Some(1000),
            released_at: Some(2000),
            ..Default::default()
        };

        let mut en_music = jp_music.clone();
        en_music.published_at = Some(1500); // Different from JP
        en_music.released_at = Some(2500); // Different from JP

        let mut region_data = std::collections::HashMap::new();
        region_data.insert(ServerRegion::Jp, vec![jp_music]);
        region_data.insert(ServerRegion::En, vec![en_music]);

        let merged = merge_musics(region_data);
        assert_eq!(merged.len(), 1);

        let music = &merged[0];
        assert!(matches!(music.published_at, UnifiedValue::Regional(_)));
        assert!(matches!(music.released_at, UnifiedValue::Regional(_)));
    }

    #[test]
    fn test_available_regions() {
        let jp_music = MusicElement {
            id: Some(1),
            published_at: Some(1000),
            released_at: Some(2000),
            ..Default::default()
        };

        let mut region_data = std::collections::HashMap::new();
        region_data.insert(ServerRegion::Jp, vec![jp_music.clone()]);
        region_data.insert(ServerRegion::En, vec![jp_music]);

        let merged = merge_musics(region_data);
        let music = &merged[0];

        assert_eq!(music.available_regions.len(), 2);
        assert!(music.available_regions.contains(&ServerRegion::Jp));
        assert!(music.available_regions.contains(&ServerRegion::En));
    }
}

// Default implementation for MusicElement to make tests work
impl Default for MusicElement {
    fn default() -> Self {
        Self {
            id: None,
            seq: None,
            release_condition_id: None,
            categories: None,
            title: None,
            pronunciation: None,
            creator_artist_id: None,
            lyricist: None,
            composer: None,
            arranger: None,
            dancer_count: None,
            self_dancer_position: None,
            assetbundle_name: None,
            live_talk_background_assetbundle_name: None,
            published_at: None,
            released_at: None,
            live_stage_id: None,
            filler_sec: None,
            is_newly_written_music: None,
            is_full_length: None,
            music_collaboration_id: None,
            infos: None,
        }
    }
}

/// Run the music merge with all 5 regions and export to file
/// Call this from anywhere to test the merge functionality
pub async fn run_merge_test_and_export(
    base_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<Vec<UniversalMusic>, crate::error::AppError> {
    use std::collections::HashMap;

    // Region folder mappings
    let region_folders = [
        (ServerRegion::Jp, "haruki-sekai-master"),
        (ServerRegion::En, "haruki-sekai-en-master"),
        (ServerRegion::Tw, "haruki-sekai-tc-master"),
        (ServerRegion::Kr, "haruki-sekai-kr-master"),
        (ServerRegion::Cn, "haruki-sekai-sc-master"),
    ];

    println!("=== Music Merge Test ===\n");

    // Load music data from all regions
    let mut region_data: HashMap<ServerRegion, Vec<MusicElement>> = HashMap::new();

    for (region, folder) in &region_folders {
        let master_dir = base_path.join(folder).join("master");
        let master_dir_str = master_dir.to_string_lossy().to_string();

        match crate::universal_master::loader::load_master_data_file::<Vec<MusicElement>>(
            &master_dir_str,
            "musics",
        )
        .await
        {
            Ok(data) => {
                println!(
                    "✓ Loaded {} musics from {} ({})",
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

    println!("\nLoaded data from {} regions", region_data.len());

    if region_data.is_empty() {
        return Err(crate::error::AppError::ParseError(
            "No data loaded from any region".to_string(),
        ));
    }

    // Merge all music data
    let merged = merge_musics(region_data);
    println!("Merged into {} universal musics", merged.len());

    // Export to JSON file
    let json_content = serde_json::to_string_pretty(&merged)
        .map_err(|e| crate::error::AppError::ParseError(format!("Failed to serialize: {}", e)))?;

    std::fs::write(output_path, &json_content)
        .map_err(|e| crate::error::AppError::IoError(format!("Failed to write file: {}", e)))?;

    println!("\n✓ Exported to: {}", output_path.display());
    println!("  File size: {} bytes", json_content.len());

    // Print statistics
    let mut uniform_count = 0;
    let mut regional_count = 0;

    for music in &merged {
        match &music.published_at {
            UnifiedValue::Uniform(_) => uniform_count += 1,
            UnifiedValue::Regional(_) => regional_count += 1,
        }
    }

    println!("\n=== Statistics ===");
    println!("Total musics: {}", merged.len());
    println!("Uniform timestamps: {}", uniform_count);
    println!("Regional timestamps: {}", regional_count);

    // Print first 5 examples
    println!("\n=== First 5 Examples ===");
    for (i, music) in merged.iter().take(5).enumerate() {
        println!("\n[{}] ID: {} - {:?}", i + 1, music.id, music.title);
        println!(
            "    Regions: {:?}",
            music
                .available_regions
                .iter()
                .map(|r| r.as_str().to_uppercase())
                .collect::<Vec<_>>()
        );
        match &music.published_at {
            UnifiedValue::Uniform(ts) => println!("    Published: {} (uniform)", ts),
            UnifiedValue::Regional(r) => {
                println!("    Published: (regional)");
                for (region, ts) in r.iter() {
                    println!("      {}: {}", region.as_str().to_uppercase(), ts);
                }
            }
        }
    }

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_merge_all_regions_and_export() {
        let base_path = Path::new("Data/master");
        let output_path = Path::new("Data/merged_musics_test.json");

        // Skip if Data/master doesn't exist (CI environment)
        if !base_path.exists() {
            println!("Skipping integration test: Data/master not found");
            return;
        }

        let result = run_merge_test_and_export(base_path, output_path).await;

        match result {
            Ok(merged) => {
                assert!(!merged.is_empty(), "Should have merged music data");
                println!("\n=== Test Passed ===");
            }
            Err(e) => {
                panic!("Merge test failed: {}", e);
            }
        }
    }
}

//! UniversalEvent - Merged event data across all regions

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;
use crate::models::events::{EventElement, EventRankingRewardRange, EventType, Unit};

use super::merger::{collect_by_id, get_first_value, merge_field, Mergeable};
use super::types::UnifiedValue;

impl Mergeable for EventElement {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalEvent {
    pub id: i64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<EventType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bgm_assetbundle_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_count_leader_character_play: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_ranking_reward_ranges: Option<Vec<EventRankingRewardRange>>,

    // Regional fields
    /// Localized event name
    pub name: UnifiedValue<String>,

    pub start_at: UnifiedValue<i64>,
    pub aggregate_at: UnifiedValue<i64>,
    pub ranking_announce_at: UnifiedValue<i64>,
    pub distribution_start_at: UnifiedValue<i64>,
    pub closed_at: UnifiedValue<i64>,
    pub distribution_end_at: UnifiedValue<i64>,

    /// Universal field: same virtual live ID across all regions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virtual_live_id: Option<i64>,

    pub available_regions: Vec<ServerRegion>,
}

impl UniversalEvent {
    pub fn from_regional(regional: &super::types::RegionalData<EventElement>) -> Option<Self> {
        let id = get_first_value(regional, |e| e.id)?;
        let available_regions = regional.available_regions();

        // Universal fields
        let event_type = get_first_value(regional, |e| e.event_type.clone());
        let assetbundle_name = get_first_value(regional, |e| e.assetbundle_name.clone());
        let bgm_assetbundle_name = get_first_value(regional, |e| e.bgm_assetbundle_name.clone());
        let unit = get_first_value(regional, |e| e.unit.clone());
        let is_count_leader_character_play =
            get_first_value(regional, |e| e.is_count_leader_character_play.flatten());
        let event_ranking_reward_ranges =
            get_first_value(regional, |e| e.event_ranking_reward_ranges.clone());

        // Regional fields
        let name = merge_field(regional, |e| e.name.clone());
        let start_at = merge_field(regional, |e| e.start_at);
        let aggregate_at = merge_field(regional, |e| e.aggregate_at);
        let ranking_announce_at = merge_field(regional, |e| e.ranking_announce_at);
        let distribution_start_at = merge_field(regional, |e| e.distribution_start_at);
        let closed_at = merge_field(regional, |e| e.closed_at);
        let distribution_end_at = merge_field(regional, |e| e.distribution_end_at);
        let virtual_live_id = get_first_value(regional, |e| e.virtual_live_id.flatten());

        Some(UniversalEvent {
            id,
            event_type,
            assetbundle_name,
            bgm_assetbundle_name,
            unit,
            is_count_leader_character_play,
            event_ranking_reward_ranges,
            name,
            start_at,
            aggregate_at,
            ranking_announce_at,
            distribution_start_at,
            closed_at,
            distribution_end_at,
            virtual_live_id,
            available_regions,
        })
    }
}

pub fn merge_events(
    region_data: std::collections::HashMap<ServerRegion, Vec<EventElement>>,
) -> Vec<UniversalEvent> {
    let by_id = collect_by_id(region_data);
    let mut result: Vec<UniversalEvent> = by_id
        .values()
        .filter_map(UniversalEvent::from_regional)
        .collect();
    result.sort_by_key(|e| e.id);
    result
}

/// Run the event merge with all 5 regions and export to file
pub async fn run_merge_test_and_export(
    base_path: &std::path::Path,
    output_path: &std::path::Path,
) -> Result<Vec<UniversalEvent>, crate::error::AppError> {
    use std::collections::HashMap;

    let region_folders = [
        (ServerRegion::Jp, "haruki-sekai-master"),
        (ServerRegion::En, "haruki-sekai-en-master"),
        (ServerRegion::Tw, "haruki-sekai-tc-master"),
        (ServerRegion::Kr, "haruki-sekai-kr-master"),
        (ServerRegion::Cn, "haruki-sekai-sc-master"),
    ];

    println!("=== Event Merge Test ===\n");

    let mut region_data: HashMap<ServerRegion, Vec<EventElement>> = HashMap::new();

    for (region, folder) in &region_folders {
        let master_dir = base_path.join(folder).join("master");
        let master_dir_str = master_dir.to_string_lossy().to_string();

        match crate::universal_master::loader::load_master_data_file::<Vec<EventElement>>(
            &master_dir_str,
            "events",
        )
        .await
        {
            Ok(data) => {
                println!(
                    "✓ Loaded {} events from {} ({})",
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

    let merged = merge_events(region_data);
    println!("Merged into {} universal events", merged.len());

    // Export to JSON
    let json_content = serde_json::to_string_pretty(&merged)
        .map_err(|e| crate::error::AppError::ParseError(format!("Failed to serialize: {}", e)))?;

    std::fs::write(output_path, &json_content)
        .map_err(|e| crate::error::AppError::IoError(format!("Failed to write file: {}", e)))?;

    println!("\n✓ Exported to: {}", output_path.display());

    // Verify virtual_live_id is Universal (not wrapped in UnifiedValue)
    let events_with_vlid = merged
        .iter()
        .filter(|e| e.virtual_live_id.is_some())
        .count();
    let events_without_vlid = merged
        .iter()
        .filter(|e| e.virtual_live_id.is_none())
        .count();

    println!("\n=== virtual_live_id Statistics ===");
    println!("Events with virtualLiveId: {}", events_with_vlid);
    println!("Events without virtualLiveId: {}", events_without_vlid);
    println!(
        "✓ All virtualLiveId values are treated as Universal (taken from first available region)"
    );

    Ok(merged)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn test_merge_events_and_verify_virtual_live_id() {
        let base_path = Path::new("Data/master");
        let output_path = Path::new("Data/merged_events_test.json");

        // Skip if Data/master doesn't exist (CI environment)
        if !base_path.exists() {
            println!("Skipping integration test: Data/master not found");
            return;
        }

        let result = run_merge_test_and_export(base_path, output_path).await;

        match result {
            Ok(merged) => {
                assert!(!merged.is_empty(), "Should have merged event data");

                // Verify virtual_live_id is a simple Option<i64> (Universal field)
                // It should NOT be wrapped in UnifiedValue
                for event in &merged {
                    // virtual_live_id is Option<i64>, confirming it's treated as Universal
                    if let Some(vlid) = event.virtual_live_id {
                        assert!(vlid > 0, "virtualLiveId should be positive when present");
                    }
                }

                println!("\n=== Test Passed ===");
            }
            Err(e) => {
                panic!("Merge test failed: {}", e);
            }
        }
    }
}

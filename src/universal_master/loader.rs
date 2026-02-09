//! Data loader for reading master data from different regions

use std::collections::HashMap;
use std::path::Path;

use serde::de::DeserializeOwned;
use tracing::{info, warn};

use crate::config::ServerRegion;
use crate::error::AppError;

/// Load master data JSON file for a specific data key from a region's master directory
pub async fn load_master_data_file<T: DeserializeOwned>(
    master_dir: &str,
    data_key: &str,
) -> Result<T, AppError> {
    let file_path = Path::new(master_dir).join(format!("{}.json", data_key));
    let content = tokio::fs::read(&file_path)
        .await
        .map_err(|e| AppError::IoError(e.to_string()))?;
    let data: T = sonic_rs::from_slice(&content)
        .map_err(|e| AppError::ParseError(format!("Failed to parse {}: {}", data_key, e)))?;
    Ok(data)
}

/// Load master data from all enabled regions
/// Returns a map of region -> data for the specified data key
pub async fn load_all_regions<T: DeserializeOwned>(
    region_master_dirs: &HashMap<ServerRegion, String>,
    data_key: &str,
) -> HashMap<ServerRegion, T> {
    let mut results = HashMap::new();

    for (region, master_dir) in region_master_dirs {
        match load_master_data_file::<T>(master_dir, data_key).await {
            Ok(data) => {
                info!(
                    "Loaded {} from {} region",
                    data_key,
                    region.as_str().to_uppercase()
                );
                results.insert(*region, data);
            }
            Err(e) => {
                warn!(
                    "Failed to load {} from {} region: {}",
                    data_key,
                    region.as_str().to_uppercase(),
                    e
                );
            }
        }
    }

    results
}

/// Build a map of region -> master_dir from the server configs
pub fn build_region_master_dirs(
    servers: &HashMap<ServerRegion, crate::config::ServerConfig>,
) -> HashMap<ServerRegion, String> {
    servers
        .iter()
        .filter(|(_, config)| config.enabled && !config.master_dir.is_empty())
        .map(|(region, config)| (*region, config.master_dir.clone()))
        .collect()
}

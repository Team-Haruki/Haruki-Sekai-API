//! Core types for UniversalMasterData

use serde::{Deserialize, Serialize};

use crate::config::ServerRegion;

/// Holds region-specific values for a field that differs across regions.
/// Uses `#[serde(skip_serializing_if = "Option::is_none")]` to omit None values in serialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionalData<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jp: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub en: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tw: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kr: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cn: Option<T>,
}

impl<T> Default for RegionalData<T> {
    fn default() -> Self {
        Self {
            jp: None,
            en: None,
            tw: None,
            kr: None,
            cn: None,
        }
    }
}

impl<T> RegionalData<T> {
    /// Create a new RegionalData with all None values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set value for a specific region
    pub fn set(&mut self, region: ServerRegion, value: T) {
        match region {
            ServerRegion::Jp => self.jp = Some(value),
            ServerRegion::En => self.en = Some(value),
            ServerRegion::Tw => self.tw = Some(value),
            ServerRegion::Kr => self.kr = Some(value),
            ServerRegion::Cn => self.cn = Some(value),
        }
    }

    /// Get value for a specific region
    pub fn get(&self, region: ServerRegion) -> Option<&T> {
        match region {
            ServerRegion::Jp => self.jp.as_ref(),
            ServerRegion::En => self.en.as_ref(),
            ServerRegion::Tw => self.tw.as_ref(),
            ServerRegion::Kr => self.kr.as_ref(),
            ServerRegion::Cn => self.cn.as_ref(),
        }
    }

    /// Get list of regions that have values
    pub fn available_regions(&self) -> Vec<ServerRegion> {
        let mut regions = Vec::new();
        if self.jp.is_some() {
            regions.push(ServerRegion::Jp);
        }
        if self.en.is_some() {
            regions.push(ServerRegion::En);
        }
        if self.tw.is_some() {
            regions.push(ServerRegion::Tw);
        }
        if self.kr.is_some() {
            regions.push(ServerRegion::Kr);
        }
        if self.cn.is_some() {
            regions.push(ServerRegion::Cn);
        }
        regions
    }

    /// Iterate over present (region, value) pairs
    pub fn iter(&self) -> impl Iterator<Item = (ServerRegion, &T)> {
        [
            (ServerRegion::Jp, &self.jp),
            (ServerRegion::En, &self.en),
            (ServerRegion::Tw, &self.tw),
            (ServerRegion::Kr, &self.kr),
            (ServerRegion::Cn, &self.cn),
        ]
        .into_iter()
        .filter_map(|(region, opt)| opt.as_ref().map(|v| (region, v)))
    }
}

impl<T: Clone + PartialEq> RegionalData<T> {
    /// Check if all present values are the same
    pub fn is_uniform(&self) -> bool {
        let values: Vec<&T> = [&self.jp, &self.en, &self.tw, &self.kr, &self.cn]
            .into_iter()
            .filter_map(|v| v.as_ref())
            .collect();

        if values.len() <= 1 {
            return true;
        }

        let first = values[0];
        values.iter().skip(1).all(|v| *v == first)
    }

    /// Get the uniform value if all present values are the same
    pub fn get_uniform(&self) -> Option<T> {
        if self.is_uniform() {
            [&self.jp, &self.en, &self.tw, &self.kr, &self.cn]
                .into_iter()
                .find_map(|v| v.clone())
        } else {
            None
        }
    }
}

/// Represents a value that may be uniform across all regions or differ per region.
/// Serializes as either the raw value (uniform) or as a regional object (differs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UnifiedValue<T> {
    /// Same value across all enabled regions
    Uniform(T),
    /// Different values per region
    Regional(RegionalData<T>),
}

impl<T: Clone + PartialEq> UnifiedValue<T> {
    /// Create from RegionalData, automatically choosing Uniform or Regional
    pub fn from_regional(data: RegionalData<T>) -> Self {
        match data.get_uniform() {
            Some(value) => UnifiedValue::Uniform(value),
            None => UnifiedValue::Regional(data),
        }
    }
}

/// Config for UniversalMasterData sources
#[derive(Debug, Clone, Deserialize)]
pub struct UniversalMasterConfig {
    /// Whether UniversalMasterData is enabled
    #[serde(default)]
    pub enabled: bool,
    /// Data source paths per region (uses servers.{region}.master_dir if not specified)
    #[serde(default)]
    pub region_sources: std::collections::HashMap<ServerRegion, DataSource>,
}

/// Data source for a region
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DataSource {
    /// Local file path
    File { path: String },
    /// Remote URL
    Url { url: String },
}

impl Default for UniversalMasterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            region_sources: std::collections::HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regional_data_uniform() {
        let mut data: RegionalData<i64> = RegionalData::new();
        data.jp = Some(100);
        data.en = Some(100);
        data.tw = Some(100);
        assert!(data.is_uniform());
        assert_eq!(data.get_uniform(), Some(100));
    }

    #[test]
    fn test_regional_data_not_uniform() {
        let mut data: RegionalData<i64> = RegionalData::new();
        data.jp = Some(100);
        data.en = Some(200);
        assert!(!data.is_uniform());
        assert_eq!(data.get_uniform(), None);
    }

    #[test]
    fn test_unified_value_from_uniform() {
        let mut data: RegionalData<i64> = RegionalData::new();
        data.jp = Some(100);
        data.en = Some(100);
        let unified = UnifiedValue::from_regional(data);
        assert!(matches!(unified, UnifiedValue::Uniform(100)));
    }

    #[test]
    fn test_unified_value_from_regional() {
        let mut data: RegionalData<i64> = RegionalData::new();
        data.jp = Some(100);
        data.en = Some(200);
        let unified = UnifiedValue::from_regional(data);
        assert!(matches!(unified, UnifiedValue::Regional(_)));
    }

    #[test]
    fn test_available_regions() {
        let mut data: RegionalData<i64> = RegionalData::new();
        data.jp = Some(100);
        data.tw = Some(200);
        data.cn = Some(300);
        let regions = data.available_regions();
        assert_eq!(regions.len(), 3);
        assert!(regions.contains(&ServerRegion::Jp));
        assert!(regions.contains(&ServerRegion::Tw));
        assert!(regions.contains(&ServerRegion::Cn));
    }
}

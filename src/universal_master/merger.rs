//! Merge logic for combining region-specific data into unified format

use std::collections::HashMap;

use crate::config::ServerRegion;

use super::types::RegionalData;

/// Trait for items that can be merged across regions
pub trait Mergeable {
    /// The ID type used to identify matching items across regions
    type Id: std::hash::Hash + Eq + Clone;

    /// Get the unique identifier for this item
    fn id(&self) -> Self::Id;
}

/// Collect items from all regions into a map of id -> RegionalData<Item>
pub fn collect_by_id<T: Mergeable + Clone>(
    region_data: HashMap<ServerRegion, Vec<T>>,
) -> HashMap<T::Id, RegionalData<T>> {
    let mut result: HashMap<T::Id, RegionalData<T>> = HashMap::new();

    for (region, items) in region_data {
        for item in items {
            let id = item.id();
            result
                .entry(id)
                .or_insert_with(RegionalData::new)
                .set(region, item);
        }
    }

    result
}

/// Helper to merge a specific field across regions
/// Returns UnifiedValue::Uniform if all values are the same, otherwise Regional
pub fn merge_field<T: Clone + PartialEq, F, R>(
    regional_items: &RegionalData<R>,
    extractor: F,
) -> super::types::UnifiedValue<T>
where
    F: Fn(&R) -> Option<T>,
{
    let mut field_data: RegionalData<T> = RegionalData::new();

    if let Some(item) = &regional_items.jp {
        if let Some(val) = extractor(item) {
            field_data.jp = Some(val);
        }
    }
    if let Some(item) = &regional_items.en {
        if let Some(val) = extractor(item) {
            field_data.en = Some(val);
        }
    }
    if let Some(item) = &regional_items.tw {
        if let Some(val) = extractor(item) {
            field_data.tw = Some(val);
        }
    }
    if let Some(item) = &regional_items.kr {
        if let Some(val) = extractor(item) {
            field_data.kr = Some(val);
        }
    }
    if let Some(item) = &regional_items.cn {
        if let Some(val) = extractor(item) {
            field_data.cn = Some(val);
        }
    }

    super::types::UnifiedValue::from_regional(field_data)
}

/// Get the first available value from a RegionalData (for uniform fields)
pub fn get_first_value<T: Clone, F, R>(regional_items: &RegionalData<R>, extractor: F) -> Option<T>
where
    F: Fn(&R) -> Option<T>,
{
    // Priority order: jp, en, tw, kr, cn
    if let Some(item) = &regional_items.jp {
        if let Some(val) = extractor(item) {
            return Some(val);
        }
    }
    if let Some(item) = &regional_items.en {
        if let Some(val) = extractor(item) {
            return Some(val);
        }
    }
    if let Some(item) = &regional_items.tw {
        if let Some(val) = extractor(item) {
            return Some(val);
        }
    }
    if let Some(item) = &regional_items.kr {
        if let Some(val) = extractor(item) {
            return Some(val);
        }
    }
    if let Some(item) = &regional_items.cn {
        if let Some(val) = extractor(item) {
            return Some(val);
        }
    }
    None
}

/// Merge optional regional data (e.g., infos field only in TW/KR/CN)
/// Returns None if no region has the field, otherwise returns RegionalData
pub fn merge_optional_regional_field<T: Clone, F, R>(
    regional_items: &RegionalData<R>,
    extractor: F,
) -> Option<RegionalData<T>>
where
    F: Fn(&R) -> Option<T>,
{
    let mut field_data: RegionalData<T> = RegionalData::new();
    let mut has_any = false;

    if let Some(item) = &regional_items.tw {
        if let Some(val) = extractor(item) {
            field_data.tw = Some(val);
            has_any = true;
        }
    }
    if let Some(item) = &regional_items.kr {
        if let Some(val) = extractor(item) {
            field_data.kr = Some(val);
            has_any = true;
        }
    }
    if let Some(item) = &regional_items.cn {
        if let Some(val) = extractor(item) {
            field_data.cn = Some(val);
            has_any = true;
        }
    }

    if has_any {
        Some(field_data)
    } else {
        None
    }
}

/// Get value from JP region only (for fields where JP data is authoritative)
pub fn get_jp_value<T: Clone, F, R>(regional_items: &RegionalData<R>, extractor: F) -> Option<T>
where
    F: Fn(&R) -> Option<T>,
{
    if let Some(item) = &regional_items.jp {
        return extractor(item);
    }
    None
}

/// Merge optional regional field across ALL 5 regions
/// Returns None if no region has the field, otherwise returns RegionalData
pub fn merge_optional_regional_field_all<T: Clone, F, R>(
    regional_items: &RegionalData<R>,
    extractor: F,
) -> Option<RegionalData<T>>
where
    F: Fn(&R) -> Option<T>,
{
    let mut field_data: RegionalData<T> = RegionalData::new();
    let mut has_any = false;

    if let Some(item) = &regional_items.jp {
        if let Some(val) = extractor(item) {
            field_data.jp = Some(val);
            has_any = true;
        }
    }
    if let Some(item) = &regional_items.en {
        if let Some(val) = extractor(item) {
            field_data.en = Some(val);
            has_any = true;
        }
    }
    if let Some(item) = &regional_items.tw {
        if let Some(val) = extractor(item) {
            field_data.tw = Some(val);
            has_any = true;
        }
    }
    if let Some(item) = &regional_items.kr {
        if let Some(val) = extractor(item) {
            field_data.kr = Some(val);
            has_any = true;
        }
    }
    if let Some(item) = &regional_items.cn {
        if let Some(val) = extractor(item) {
            field_data.cn = Some(val);
            has_any = true;
        }
    }

    if has_any {
        Some(field_data)
    } else {
        None
    }
}

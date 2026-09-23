use serde::{Deserialize, Serialize};

pub type Omikuji = Vec<OmikujiElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmikujiElement {
    pub id: Option<i64>,

    pub omikuji_group_id: Option<i64>,

    pub unit: Option<String>,

    pub fortune_type: Option<String>,

    pub summary: Option<String>,

    pub title1: Option<String>,

    pub description1: Option<String>,

    pub title2: Option<String>,

    pub description2: Option<String>,

    pub title3: Option<String>,

    pub description3: Option<String>,

    pub unit_assetbundle_name: Option<String>,

    pub fortune_assetbundle_name: Option<String>,

    pub omikuji_cover_assetbundle_name: Option<String>,

    pub unit_file_path: Option<String>,

    pub fortune_file_path: Option<String>,

    pub omikuji_cover_file_path: Option<String>,
}

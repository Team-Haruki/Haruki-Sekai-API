use serde::{Deserialize, Serialize};

pub type Mysekaihousingcompetition = Vec<MysekaihousingcompetitionElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaihousingcompetitionElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub description: Option<String>,

    pub submit_start_at: Option<i64>,

    pub review_start_at: Option<i64>,

    pub submit_end_at: Option<i64>,

    pub aggregate_at: Option<i64>,

    pub background_image_assetbundle_file_name: Option<String>,

    pub back_number_accent_color_code: Option<String>,
}

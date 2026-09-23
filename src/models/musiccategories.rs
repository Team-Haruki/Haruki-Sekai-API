use serde::{Deserialize, Serialize};

pub type Musiccategorie = Vec<MusiccategorieElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusiccategorieElement {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_category_name: Option<String>,

    pub music_asset_variant_id: Option<i64>,

    pub published_at: Option<i64>,
}

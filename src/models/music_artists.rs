use serde::{Deserialize, Serialize};

pub type MusicArtist = Vec<MusicArtistElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicArtistElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,
}

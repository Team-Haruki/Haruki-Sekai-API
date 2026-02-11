//! Database entity for UniversalMusic
//!
//! Stores merged music data with JSONB fields for complex structures

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "universal_musics")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,

    /// JSONB: Vec<CategoryElement>
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub categories: Option<serde_json::Value>,

    #[sea_orm(nullable)]
    pub title: Option<String>,

    #[sea_orm(nullable)]
    pub pronunciation: Option<String>,

    #[sea_orm(nullable)]
    pub creator_artist_id: Option<i64>,

    #[sea_orm(nullable)]
    pub lyricist: Option<String>,

    #[sea_orm(nullable)]
    pub composer: Option<String>,

    #[sea_orm(nullable)]
    pub arranger: Option<String>,

    #[sea_orm(nullable)]
    pub dancer_count: Option<i64>,

    #[sea_orm(nullable)]
    pub self_dancer_position: Option<i64>,

    #[sea_orm(nullable)]
    pub assetbundle_name: Option<String>,

    #[sea_orm(nullable)]
    pub live_talk_background_assetbundle_name: Option<String>,

    /// JSONB: UnifiedValue<i64>
    #[sea_orm(column_type = "JsonBinary")]
    pub published_at: serde_json::Value,

    /// JSONB: UnifiedValue<i64>
    #[sea_orm(column_type = "JsonBinary")]
    pub released_at: serde_json::Value,

    #[sea_orm(nullable)]
    pub live_stage_id: Option<i64>,

    #[sea_orm(nullable)]
    pub filler_sec: Option<f64>,

    #[sea_orm(nullable)]
    pub is_newly_written_music: Option<bool>,

    #[sea_orm(nullable)]
    pub is_full_length: Option<bool>,

    #[sea_orm(nullable)]
    pub music_collaboration_id: Option<i64>,

    /// JSONB: RegionalData<Vec<Info>>
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub infos: Option<serde_json::Value>,

    /// JSONB: Vec<ServerRegion>
    #[sea_orm(column_type = "JsonBinary")]
    pub available_regions: serde_json::Value,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    /// Convert from UniversalMusic to database model
    pub fn from_universal(music: &crate::universal_master::music::UniversalMusic) -> Self {
        Self {
            id: music.id,
            categories: music
                .categories
                .as_ref()
                .map(|c| serde_json::to_value(c).unwrap_or(serde_json::Value::Null)),
            title: music.title.clone(),
            pronunciation: music.pronunciation.clone(),
            creator_artist_id: music.creator_artist_id,
            lyricist: music.lyricist.clone(),
            composer: music.composer.clone(),
            arranger: music.arranger.clone(),
            dancer_count: music.dancer_count,
            self_dancer_position: music.self_dancer_position,
            assetbundle_name: music.assetbundle_name.clone(),
            live_talk_background_assetbundle_name: music
                .live_talk_background_assetbundle_name
                .clone(),
            published_at: serde_json::to_value(&music.published_at)
                .unwrap_or(serde_json::Value::Null),
            released_at: serde_json::to_value(&music.released_at)
                .unwrap_or(serde_json::Value::Null),
            live_stage_id: music.live_stage_id,
            filler_sec: music.filler_sec,
            is_newly_written_music: music.is_newly_written_music,
            is_full_length: music.is_full_length,
            music_collaboration_id: music.music_collaboration_id,
            infos: music
                .infos
                .as_ref()
                .map(|i| serde_json::to_value(i).unwrap_or(serde_json::Value::Null)),
            available_regions: serde_json::to_value(&music.available_regions)
                .unwrap_or(serde_json::Value::Null),
        }
    }
}

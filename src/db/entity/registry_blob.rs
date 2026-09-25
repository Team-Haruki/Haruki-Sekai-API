use sea_orm::entity::prelude::*;

/// One content-addressed master file of the registry blob store
/// (`registry.blob_store: pg`): the bytes whose SHA-256 is `sha256`,
/// stored `encoding`-compressed. Shared by every region and version that
/// lists the same digest.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "registry_blobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub sha256: String,
    /// Uncompressed size in bytes (the manifest's `size`).
    pub size: i64,
    /// `zstd` (one frame, content checksum on).
    pub encoding: String,
    /// Compressed size in bytes.
    pub stored_size: i64,
    pub content: Vec<u8>,
    pub created_at: DateTimeUtc,
    /// Last time a publish or import referenced this blob; garbage
    /// collection only removes unreferenced blobs older than the grace period.
    #[sea_orm(indexed)]
    pub last_seen_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

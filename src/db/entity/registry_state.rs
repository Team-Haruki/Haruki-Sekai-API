use sea_orm::entity::prelude::*;

/// One registry state document: a region's current manifest, a manifest
/// snapshot (`name` = contentHash), the app-identity override or the
/// music_metas pointer, keyed by `(region, kind, name)`.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "registry_state")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub region: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub name: String,
    #[sea_orm(column_type = "JsonBinary")]
    pub value: Json,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

use sea_orm::entity::prelude::*;

/// One publish of a region (a `PublishRecord`), appended in publish order.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "registry_publish_history")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(indexed)]
    pub region: String,
    #[sea_orm(column_type = "JsonBinary")]
    pub record: Json,
    pub recorded_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

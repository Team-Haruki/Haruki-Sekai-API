use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sekai_user_servers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub server: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::sekai_user::Entity",
        from = "Column::UserId",
        to = "super::sekai_user::Column::Id"
    )]
    SekaiUser,
}

impl Related<super::sekai_user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SekaiUser.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

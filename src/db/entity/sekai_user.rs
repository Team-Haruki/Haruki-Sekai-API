use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sekai_users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub credential: String,
    #[sea_orm(default_value = "")]
    pub remark: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::sekai_user_server::Entity")]
    SekaiUserServers,
}

impl Related<super::sekai_user_server::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::SekaiUserServers.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

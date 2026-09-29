use serde::{Deserialize, Serialize};

pub type Mysekaiblueprintterm = Vec<MysekaiblueprinttermElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiblueprinttermElement {
    pub id: Option<i64>,

    pub mysekai_blueprint_id: Option<i64>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,

    pub mysekai_blueprint_term_tab_type: Option<String>,

    pub mysekai_blueprint_term_mysekai_material_cost_group_id: Option<i64>,

    pub craft_limit: Option<i64>,
}

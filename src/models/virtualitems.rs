use serde::{Deserialize, Serialize};

pub type Virtualitem = Vec<VirtualitemElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualitemElement {
    pub id: Option<i64>,

    pub virtual_item_category: Option<String>,

    pub seq: Option<i64>,

    pub priority: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub cost_virtual_coin: Option<i64>,

    pub cost_jewel: Option<i64>,

    pub effect_assetbundle_name: Option<String>,

    pub effect_expression_type: Option<String>,

    pub virtual_item_label_type: Option<String>,

    pub start_at: Option<i64>,

    pub end_at: Option<i64>,

    pub game_character_unit_id: Option<i64>,

    pub unit: Option<String>,

    pub sub_game_character_id: Option<i64>,

    pub virtual_item_type: Option<String>,
}

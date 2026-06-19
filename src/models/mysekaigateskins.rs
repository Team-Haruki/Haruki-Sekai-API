use serde::{Deserialize, Serialize};

pub type Mysekaigateskin = Vec<MysekaigateskinElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigateskinElement {
    pub id: Option<i64>,

    pub mysekai_gate_skin_type: Option<String>,

    pub mysekai_gate_skin_type_id: Option<i64>,

    pub mysekai_gate_material_group_id: Option<i64>,
}

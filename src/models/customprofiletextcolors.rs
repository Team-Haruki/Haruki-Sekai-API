use serde::{Deserialize, Serialize};

pub type Customprofiletextcolor = Vec<CustomprofiletextcolorElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomprofiletextcolorElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub color_code: Option<String>,
}

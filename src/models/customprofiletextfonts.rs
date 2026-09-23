use serde::{Deserialize, Serialize};

pub type Customprofiletextfont = Vec<CustomprofiletextfontElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomprofiletextfontElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub font_name: Option<String>,

    pub assetbundle_name: Option<String>,
}

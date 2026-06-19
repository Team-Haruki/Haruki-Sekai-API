use serde::{Deserialize, Serialize};

pub type Mysekaigateunitskin = Vec<MysekaigateunitskinElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigateunitskinElement {
    pub id: Option<i64>,

    pub unit: Option<String>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,
}

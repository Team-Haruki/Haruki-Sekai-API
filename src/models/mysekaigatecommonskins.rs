use serde::{Deserialize, Serialize};

pub type Mysekaigatecommonskin = Vec<MysekaigatecommonskinElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaigatecommonskinElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,
}

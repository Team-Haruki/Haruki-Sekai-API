use serde::{Deserialize, Serialize};

pub type Mysekaishop = Vec<MysekaishopElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaishopElement {
    pub id: Option<i64>,

    pub mysekai_shop_type: Option<String>,

    pub seq: Option<i64>,

    pub resource_box_id: Option<i64>,

    pub mysekai_shop_exchange_limit_type: Option<String>,

    pub mysekai_shop_exchange_limit_value: Option<i64>,
}

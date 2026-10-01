use serde::{Deserialize, Serialize};

pub type Playerframepart = Vec<PlayerframepartElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerframepartElement {
    pub id: Option<i64>,

    pub seq: Option<i64>,

    pub player_frame_group_id: Option<i64>,

    pub game_character_id: Option<i64>,
}

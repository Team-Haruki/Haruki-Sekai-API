// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Mysekaifixture;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Mysekaifixture = serde_json::from_str(&json).unwrap();
// }

use serde::{Deserialize, Serialize};

pub type Mysekaifixture = Vec<MysekaifixtureElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaifixtureElement {
    pub id: Option<i64>,

    pub mysekai_fixture_type: Option<MysekaiFixtureType>,

    pub name: Option<String>,

    pub pronunciation: Option<String>,

    pub flavor_text: Option<String>,

    pub seq: Option<i64>,

    pub grid_size: Option<GridSize>,

    pub mysekai_fixture_main_genre_id: Option<i64>,

    pub mysekai_fixture_sub_genre_id: Option<i64>,

    pub mysekai_fixture_handle_type: Option<MysekaiFixtureHandleType>,

    pub mysekai_settable_site_type: Option<MysekaiSettableSiteType>,

    pub mysekai_settable_layout_type: Option<MysekaiSettableLayoutType>,

    pub mysekai_fixture_put_type: Option<MysekaiFixturePutType>,

    pub mysekai_fixture_another_colors: Option<Vec<MysekaiFixtureAnotherColor>>,

    pub mysekai_fixture_put_sound_id: Option<i64>,

    pub mysekai_fixture_footstep_id: Option<i64>,

    pub mysekai_fixture_tag_group: Option<MysekaiFixtureTagGroup>,

    pub is_assembled: Option<bool>,

    pub is_disassembled: Option<bool>,

    pub mysekai_fixture_player_action_type: Option<MysekaiFixturePlayerActionType>,

    pub is_game_character_action: Option<bool>,

    pub assetbundle_name: Option<String>,

    pub first_put_cost: Option<i64>,

    pub second_put_cost: Option<i64>,

    pub color_code: Option<String>,

    pub mysekai_fixture_game_character_group_performance_bonus_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridSize {
    pub width: Option<i64>,

    pub depth: Option<i64>,

    pub height: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiFixtureAnotherColor {
    pub texture_id: Option<i64>,

    pub color_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiFixtureHandleType {
    Block,

    #[serde(rename = "block_transparent")]
    BlockTransparent,

    Clock,

    Fence,

    #[serde(rename = "idle_timeline")]
    IdleTimeline,

    Light,

    None,

    Road,

    Windowpane,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiFixturePlayerActionType {
    Loop,

    #[serde(rename = "no_action")]
    NoAction,

    #[serde(rename = "one_shot")]
    OneShot,

    Timeline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiFixturePutType {
    None,

    #[serde(rename = "put_base")]
    PutBase,

    #[serde(rename = "put_either")]
    PutEither,

    #[serde(rename = "put_target")]
    PutTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MysekaiFixtureTagGroup {
    pub id: Option<i64>,

    pub mysekai_fixture_tag_id1: Option<i64>,

    pub mysekai_fixture_tag_id2: Option<i64>,

    pub mysekai_fixture_tag_id3: Option<i64>,

    pub mysekai_fixture_tag_id4: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiFixtureType {
    Canvas,

    Custom,

    Gate,

    #[serde(rename = "house_plant")]
    HousePlant,

    Normal,

    Plant,

    #[serde(rename = "surface_appearance")]
    SurfaceAppearance,

    System,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiSettableLayoutType {
    Floor,

    #[serde(rename = "floor_appearance")]
    FloorAppearance,

    Road,

    Rug,

    Wall,

    #[serde(rename = "wall_appearance")]
    WallAppearance,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MysekaiSettableSiteType {
    Any,

    Home,

    Room,
}

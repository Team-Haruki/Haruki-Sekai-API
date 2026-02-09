// Example code that deserializes and serializes the model.
// extern crate serde;
// #[macro_use]
// extern crate serde_derive;
// extern crate serde_json;
//
// use generated_module::Musicvocal;
//
// fn main() {
//     let json = r#"{"answer": 42}"#;
//     let model: Musicvocal = serde_json::from_str(&json).unwrap();
// }

use serde::{Serialize, Deserialize};

pub type Musicvocal = Vec<MusicvocalElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicvocalElement {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_vocal_type: Option<MusicVocalType>,

    pub seq: Option<i64>,

    pub release_condition_id: Option<i64>,

    pub caption: Option<Caption>,

    pub characters: Option<Vec<Character>>,

    pub assetbundle_name: Option<String>,

    pub archive_published_at:Option< Option<i64>>,

    pub special_season_id:Option< Option<i64>>,

    pub archive_display_type:Option< Option<ArchiveDisplayType>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveDisplayType {
    None,

    Normal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Caption {
    #[serde(rename = "「世界」ver.")]
    AmbitiousVer,

    #[serde(rename = "Another Vocal ver.")]
    AnotherVocalVer,

    #[serde(rename = "コネクトライブ(DAY1夜)ver.")]
    CaptionDay1Ver,

    #[serde(rename = "コネクトライブ(DAY2夜)ver.")]
    CaptionDay2Ver,

    #[serde(rename = "セカイver.")]
    CaptionVer,

    #[serde(rename = "COLORFUL LIVE ver.")]
    ColorfulLiveVer,

    #[serde(rename = "愚人节ver.")]
    CunningVer,

    #[serde(rename = "コネクトライブ(DAY1昼)ver.")]
    Day1Ver,

    #[serde(rename = "コネクトライブ(DAY2昼)ver.")]
    Day2Ver,

    #[serde(rename = "连线演唱会（DAY1夜）ver.")]
    FluffyDay1Ver,

    #[serde(rename = "连线演唱会（DAY2夜）ver.")]
    FluffyDay2Ver,

    #[serde(rename = "エイプリルフールver.")]
    FluffyVer,

    #[serde(rename = "虛擬歌手ver.")]
    FriskyVer,

    #[serde(rename = "虚拟歌手ver.")]
    HilariousVer,

    #[serde(rename = "ワンダーランズ×ショウタイム ver.")]
    IndecentVer,

    #[serde(rename = "「劇場版プロジェクトセカイ」ver.")]
    IndigoVer,

    #[serde(rename = "Inst.ver.")]
    InstVer,

    #[serde(rename = "Leo/need ver.")]
    LeoNeedVer,

    #[serde(rename = "「电影世界计划初音未来」ver.")]
    MagentaVer,

    #[serde(rename = "MORE MORE JUMP！ ver.")]
    MoreMoreJumpVer,

    #[serde(rename = "连线演唱会（DAY1昼）ver.")]
    PurpleDay1Ver,

    #[serde(rename = "连线演唱会（DAY2昼）ver.")]
    PurpleDay2Ver,

    #[serde(rename = "アナザーボーカルver.")]
    PurpleVer,

    #[serde(rename = "あんさんぶるスターズ！！コラボver.")]
    StickyVer,

    #[serde(rename = "コネクトライブver.")]
    TentacledVer,

    #[serde(rename = "25時、ナイトコードで。ver.")]
    The25Ver,

    #[serde(rename = "偶像梦幻祭2联动ver.")]
    The2Ver,

    #[serde(rename = "バーチャル・シンガーver.")]
    Ver,

    #[serde(rename = "Vivid BAD SQUAD ver.")]
    VividBadSquadVer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub id: Option<i64>,

    pub music_id: Option<i64>,

    pub music_vocal_id: Option<i64>,

    pub character_type: Option<CharacterType>,

    pub character_id: Option<i64>,

    pub seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterType {
    #[serde(rename = "game_character")]
    GameCharacter,

    #[serde(rename = "outside_character")]
    OutsideCharacter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicVocalType {
    #[serde(rename = "another_vocal")]
    AnotherVocal,

    #[serde(rename = "april_fool_2022")]
    AprilFool2022,

    Instrumental,

    #[serde(rename = "original_song")]
    OriginalSong,

    Sekai,

    #[serde(rename = "streaming_live")]
    StreamingLive,

    #[serde(rename = "virtual_singer")]
    VirtualSinger,
}

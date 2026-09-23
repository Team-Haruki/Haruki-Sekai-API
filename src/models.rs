pub mod areaitemlevels;
pub mod areaitems;
pub mod areas;
pub mod bonds;
pub mod bondshonors;
pub mod bondshonorwords;
pub mod boostitems;
pub mod cardcostume3ds;
pub mod cardepisodes;
pub mod cardmysekaicanvasbonuses;
pub mod cardrarities;
pub mod cards;
pub mod cardsupplies;
pub mod challengelivehighscorerewards;
pub mod character2ds;
pub mod characterarchivemysekaicharactertalkgroups;
pub mod charactermissionv2areaitems;
pub mod charactermissionv2exjsons;
pub mod charactermissionv2parametergroups;
pub mod charactermissionv2s;
pub mod characterranks;
pub mod cheerfulcarnivalteams;
pub mod costume3ds;
pub mod custommusicscoretags;
pub mod eventcards;
pub mod eventdeckbonuses;
pub mod eventexchangesummaries;
pub mod eventitems;
pub mod eventmusics;
pub mod eventraritybonusrates;
pub mod events;
pub mod eventstories;
pub mod eventstoryunits;
pub mod gachaceilitems;
pub mod gachas;
pub mod gachatickets;
pub mod gamecharacters;
pub mod gamecharacterunits;
pub mod honorgroups;
pub mod honors;
pub mod levels;
pub mod limitedtimemusics;
pub mod masterlessons;
pub mod materials;
pub mod music_artists;
pub mod musicdifficulties;
pub mod musics;
pub mod musictags;
pub mod musicvocals;
pub mod mysekaiblueprintmysekaimaterialcosts;
pub mod mysekaiblueprints;
pub mod mysekaicharactertalkconditiongroups;
pub mod mysekaicharactertalkconditions;
pub mod mysekaicharactertalkfixturecommonmysekaifixturegroups;
pub mod mysekaicharactertalkfixturecommons;
pub mod mysekaicharactertalks;
pub mod mysekaicustomfixtures;
pub mod mysekaifixturegamecharactergroupperformancebonuses;
pub mod mysekaifixturegamecharactergroups;
pub mod mysekaifixturemaingenres;
pub mod mysekaifixtureonlydisassemblematerials;
pub mod mysekaifixtures;
pub mod mysekaifixturesubgenres;
pub mod mysekaifixturetags;
pub mod mysekaigamecharacterunitgroups;
pub mod mysekaigatecharacterlotteries;
pub mod mysekaigatecommonskins;
pub mod mysekaigatelevels;
pub mod mysekaigatematerialgroups;
pub mod mysekaigates;
pub mod mysekaigateskins;
pub mod mysekaigateunitskins;
pub mod mysekaihousingcompetitions;
pub mod mysekaiitems;
pub mod mysekaimaterialgamecharacterrelations;
pub mod mysekaimaterials;
pub mod mysekaimusicrecordcategories;
pub mod mysekaimusicrecords;
pub mod mysekaiphenomenabackgroundcolors;
pub mod mysekaiphenomenas;
pub mod mysekairankreleases;
pub mod mysekaisiteharvestfixtures;
pub mod mysekaisitelayouts;
pub mod mysekaisitelevels;
pub mod ngwords;
pub mod outsidecharacters;
pub mod playerframegroups;
pub mod playerframes;
pub mod practicetickets;
pub mod resourceboxdetails;
pub mod resourceboxes;
pub mod shopitems;
pub mod skillpracticetickets;
pub mod skills;
pub mod stamps;
pub mod virtuallives;
pub mod worldbloomchapterrankingrewardranges;
pub mod worldbloomdifferentattributebonuses;
pub mod worldblooms;
pub mod worldbloomsupportdeckbonuses;
pub mod worldbloomsupportdeckuniteventlimitedbonuses;

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    fn parse<T: DeserializeOwned>(json: &str) -> Vec<T> {
        serde_json::from_str(json).expect("fixture deserializes")
    }

    #[test]
    fn character_mission_v2s_parse_cp_and_nuverse_key_orders() {
        use super::charactermissionv2s::{CharacterMissionType, Charactermissionv2Element};
        let rows: Vec<Charactermissionv2Element> = parse(
            r#"[
            {"id":1,"characterMissionType":"play_live","characterId":1,"parameterGroupId":1,
             "sentence":"ライブを{requirement}回クリア","progressSentence":"{progress}回",
             "isAchievementMission":false},
            {"characterId":2,"characterMissionType":"collect_mysekai_fixture","id":572,
             "isAchievementMission":true,"parameterGroupId":18,"progressSentence":"{progress}种",
             "sentence":"获得{requirement}种家具"}
            ]"#,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, Some(1));
        assert_eq!(
            rows[0].character_mission_type,
            Some(CharacterMissionType::PlayLive)
        );
        assert_eq!(rows[0].is_achievement_mission, Some(false));
        assert_eq!(
            rows[1].character_mission_type,
            Some(CharacterMissionType::CollectMysekaiFixture)
        );
        assert_eq!(rows[1].parameter_group_id, Some(18));
    }

    #[test]
    fn bonds_honor_words_parse() {
        use super::bondshonorwords::BondshonorwordElement;
        let rows: Vec<BondshonorwordElement> = parse(
            r#"[{"id":1,"seq":1,"bondsGroupId":1,"assetbundleName":"honor_bonds_word_001",
                 "name":"一歌&咲希","description":"desc"}]"#,
        );
        assert_eq!(rows[0].bonds_group_id, Some(1));
        assert_eq!(
            rows[0].assetbundle_name.as_deref(),
            Some("honor_bonds_word_001")
        );
    }

    #[test]
    fn world_bloom_chapter_ranking_reward_ranges_parse() {
        use super::worldbloomchapterrankingrewardranges::WorldbloomchapterrankingrewardrangeElement;
        let rows: Vec<WorldbloomchapterrankingrewardrangeElement> = parse(
            r#"[{"id":1,"eventId":112,"gameCharacterId":1,"fromRank":1,"toRank":1,
                 "isToRankBorder":false,"resourceBoxId":1}]"#,
        );
        assert_eq!(rows[0].event_id, Some(112));
        assert_eq!(rows[0].is_to_rank_border, Some(false));
        assert_eq!(rows[0].resource_box_id, Some(1));
    }

    #[test]
    fn resource_box_details_parse_nuverse_rows_without_id_or_seq() {
        use super::resourceboxdetails::{
            ResourceBoxPurpose, ResourceType, ResourceboxdetailElement,
        };
        // CN/TW/KR ship this table flat with neither `id` nor `seq`; resourceId and
        // resourceLevel are null for resources that have no id (jewel, coin, ...).
        let rows: Vec<ResourceboxdetailElement> = parse(
            r#"[
            {"resourceBoxId":1,"resourceQuantity":1,"resourceId":1,"resourceBoxPurpose":"ad_reward",
             "resourceLevel":null,"resourceType":"ad_reward_random_box"},
            {"resourceBoxId":2,"resourceQuantity":100,"resourceId":null,
             "resourceBoxPurpose":"shop_item","resourceLevel":null,"resourceType":"jewel"}
            ]"#,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].resource_box_purpose,
            Some(ResourceBoxPurpose::AdReward)
        );
        assert_eq!(rows[0].resource_level, None);
        assert_eq!(rows[1].resource_id, None);
        assert_eq!(rows[1].resource_type, Some(ResourceType::Jewel));
        assert_eq!(rows[1].resource_quantity, Some(100));
    }

    #[test]
    fn materials_parse_optional_flavor_text_and_nuverse_only_type() {
        use super::materials::{MaterialElement, MaterialType};
        let rows: Vec<MaterialElement> = parse(
            r#"[
            {"id":1,"seq":1,"name":"コイン","flavorText":"a","canUse":false,"materialType":"common",
             "flavorText2":"b","changeFlavorTextAt":1700000000000},
            {"canUse":true,"flavorText":"c","id":300,"materialType":"web_ticket","name":"票","seq":300}
            ]"#,
        );
        assert_eq!(rows[0].material_type, Some(MaterialType::Common));
        assert_eq!(rows[0].flavor_text2.as_deref(), Some("b"));
        assert_eq!(rows[0].change_flavor_text_at, Some(1_700_000_000_000));
        assert_eq!(rows[1].material_type, Some(MaterialType::WebTicket));
        assert_eq!(rows[1].flavor_text2, None);
        assert_eq!(rows[1].change_flavor_text_at, None);
    }

    #[test]
    fn practice_tickets_parse_with_and_without_character() {
        use super::practicetickets::PracticeticketElement;
        use super::skillpracticetickets::SkillpracticeticketElement;
        let rows: Vec<PracticeticketElement> = parse(
            r#"[{"id":1,"name":"練習チケット","exp":100,"flavorText":"a"},
                {"exp":200,"flavorText":"b","id":4,"name":"一歌のチケット","characterId":1}]"#,
        );
        assert_eq!(rows[0].character_id, None);
        assert_eq!(rows[1].character_id, Some(1));
        assert_eq!(rows[1].exp, Some(200));
        let rows: Vec<SkillpracticeticketElement> = parse(
            r#"[{"id":1,"name":"スキルチケット","exp":1,"flavorText":"a"},
                {"exp":2,"flavorText":"b","id":5,"name":"x","characterId":3}]"#,
        );
        assert_eq!(rows[0].character_id, None);
        assert_eq!(rows[1].character_id, Some(3));
    }

    #[test]
    fn character_mission_v2_ex_jsons_parse() {
        use super::charactermissionv2exjsons::{
            CharacterMissionExType, CharacterMissionType, Charactermissionv2ExjsonElement,
            ResourceType,
        };
        let rows: Vec<Charactermissionv2ExjsonElement> = parse(
            r#"[{"id":1,"characterMissionExType":"play_live_ex","characterMissionType":"play_live",
                 "resourceType":"material"},
                {"id":2,"characterMissionExType":"waiting_room_ex",
                 "characterMissionType":"waiting_room","resourceType":"material"}]"#,
        );
        assert_eq!(
            rows[0].character_mission_ex_type,
            Some(CharacterMissionExType::PlayLiveEx)
        );
        assert_eq!(
            rows[1].character_mission_type,
            Some(CharacterMissionType::WaitingRoom)
        );
        assert_eq!(rows[1].resource_type, Some(ResourceType::Material));
    }

    #[test]
    fn character_mission_v2_area_items_parse_character_and_unit_rows() {
        use super::charactermissionv2areaitems::{
            CharacterMissionType, Charactermissionv2AreaitemElement, Unit,
        };
        let rows: Vec<Charactermissionv2AreaitemElement> = parse(
            r#"[{"id":1,"characterMissionType":"area_item_level_up_character","areaItemId":1,
                 "characterId":1},
                {"areaItemId":30,"characterMissionType":"area_item_level_up_unit","id":31,
                 "unit":"light_sound"},
                {"id":55,"characterMissionType":"area_item_level_up_reality_world","areaItemId":55}]"#,
        );
        assert_eq!(rows[0].character_id, Some(1));
        assert_eq!(rows[0].unit, None);
        assert_eq!(rows[1].character_id, None);
        assert_eq!(rows[1].unit, Some(Unit::LightSound));
        assert_eq!(
            rows[2].character_mission_type,
            Some(CharacterMissionType::AreaItemLevelUpRealityWorld)
        );
    }
}

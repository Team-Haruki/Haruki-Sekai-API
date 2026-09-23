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
pub mod customprofilecharactericonresources;
pub mod customprofilecollectionresources;
pub mod customprofileetcresources;
pub mod customprofilegeneralbackgroundresources;
pub mod customprofilematerialresources;
pub mod customprofilememberstandingpictureresources;
pub mod customprofileplayerinforesources;
pub mod customprofileshaperesources;
pub mod customprofilestorybackgroundresources;
pub mod customprofiletextcolors;
pub mod customprofiletextfonts;
pub mod customprofileuserinterfaceiconresources;
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
pub mod musiccategories;
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
pub mod omikujis;
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
pub mod unitstoryepisodegroups;
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

    #[test]
    fn custom_profile_resources_parse_cp_and_nuverse_shapes() {
        use super::customprofilecollectionresources::CustomprofilecollectionresourceElement;
        use super::customprofilememberstandingpictureresources::CustomprofilememberstandingpictureresourceElement;
        use super::customprofiletextcolors::CustomprofiletextcolorElement;
        use super::customprofiletextfonts::CustomprofiletextfontElement;
        let rows: Vec<CustomprofilecollectionresourceElement> = parse(
            r#"[{"customProfileResourceType":"collection","id":1,"seq":10,"name":"おみくじ",
                 "pronunciation":"おみくじ","resourceLoadType":"assetbundle",
                 "resourceLoadVal":"lottery_game/new_year_2022","fileName":"Prefabs/Omikuji",
                 "customProfileResourceCollectionType":"omikuji","groupId":1}]"#,
        );
        assert_eq!(rows[0].group_id, Some(1));
        assert_eq!(
            rows[0].custom_profile_resource_collection_type.as_deref(),
            Some("omikuji")
        );
        // JP/EN rows have no pronunciation; TW/KR/CN rows do.
        let rows: Vec<CustomprofilememberstandingpictureresourceElement> = parse(
            r#"[{"customProfileResourceType":"member_standing_picture","id":1,"seq":1,"name":"ミク",
                 "resourceLoadType":"assetbundle","resourceLoadVal":"custom_profile/character",
                 "fileName":"profile_chr_miku_0001","characterId":21},
                {"customProfileResourceType":"member_standing_picture","id":2,"seq":2,"name":"初音未來",
                 "pronunciation":"みく","resourceLoadType":"assetbundle",
                 "resourceLoadVal":"custom_profile/character","fileName":"profile_chr_miku_0002",
                 "characterId":21}]"#,
        );
        assert_eq!(rows[0].pronunciation, None);
        assert_eq!(rows[1].pronunciation.as_deref(), Some("みく"));
        assert_eq!(rows[1].character_id, Some(21));
        // Nuverse key order and missing assetbundleName.
        let rows: Vec<CustomprofiletextfontElement> = parse(
            r#"[{"id":1,"name":"ピュア１","fontName":"FOT-RodinNTLGPro-DB","assetbundleName":"custom_profile/font"},
                {"fontName":"NotoSansCJKtc-Medium","id":2,"name":"純真1"}]"#,
        );
        assert_eq!(
            rows[0].assetbundle_name.as_deref(),
            Some("custom_profile/font")
        );
        assert_eq!(rows[1].assetbundle_name, None);
        assert_eq!(rows[1].font_name.as_deref(), Some("NotoSansCJKtc-Medium"));
        let rows: Vec<CustomprofiletextcolorElement> =
            parse(r##"[{"colorCode":"#444466","id":1,"seq":1}]"##);
        assert_eq!(rows[0].color_code.as_deref(), Some("#444466"));
    }

    #[test]
    fn omikujis_and_unit_story_episode_groups_parse() {
        use super::omikujis::OmikujiElement;
        use super::unitstoryepisodegroups::UnitstoryepisodegroupElement;
        let rows: Vec<OmikujiElement> = parse(
            r#"[{"description1":"必ず叶う","description2":"良好","description3":"来る",
                 "fortuneAssetbundleName":"lottery_game/new_year_2022_material",
                 "fortuneFilePath":"unsei_daikichi","fortuneType":"grate_fortune","id":1,
                 "omikujiCoverAssetbundleName":"lottery_game/new_year_2022_material",
                 "omikujiCoverFilePath":"omikuji_VIRTUAL SINGER","omikujiGroupId":1,
                 "summary":"良い年","title1":"願望","title2":"健康","title3":"待人","unit":"piapro",
                 "unitAssetbundleName":"lottery_game/new_year_2022_material",
                 "unitFilePath":"bird_VIRTUAL SINGER"}]"#,
        );
        assert_eq!(rows[0].omikuji_group_id, Some(1));
        assert_eq!(rows[0].title3.as_deref(), Some("待人"));
        assert_eq!(
            rows[0].omikuji_cover_file_path.as_deref(),
            Some("omikuji_VIRTUAL SINGER")
        );
        let rows: Vec<UnitstoryepisodegroupElement> = parse(
            r#"[{"id":1,"unit":"piapro","unitEpisodeCategory":"light_sound","outline":"o",
                 "assetbundleName":"main_lightsound_piapro"},
                {"assetbundleName":"main_idol_piapro","id":2,"outline":"o","unit":"piapro",
                 "unitEpisodeCategory":"idol"}]"#,
        );
        assert_eq!(rows[1].unit_episode_category.as_deref(), Some("idol"));
    }

    #[test]
    fn music_categories_parse_with_and_without_variant() {
        use super::musiccategories::MusiccategorieElement;
        let rows: Vec<MusiccategorieElement> = parse(
            r#"[{"id":1,"musicId":1,"musicCategoryName":"mv"},
                {"id":2,"musicId":1,"musicCategoryName":"original","musicAssetVariantId":47701,
                 "publishedAt":1788404400000}]"#,
        );
        assert_eq!(rows[0].music_asset_variant_id, None);
        assert_eq!(rows[1].music_asset_variant_id, Some(47701));
        assert_eq!(rows[1].published_at, Some(1_788_404_400_000));
        assert_eq!(rows[1].music_category_name.as_deref(), Some("original"));
    }

    #[test]
    fn previously_dropped_top_level_keys_now_deserialize() {
        use super::events::EventElement;
        use super::gachas::GachaElement;
        use super::limitedtimemusics::LimitedtimemusicElement;
        use super::musics::MusicElement;
        use super::mysekaihousingcompetitions::MysekaihousingcompetitionElement;
        use super::playerframegroups::PlayerframegroupElement;
        use super::playerframes::PlayerframeElement;
        use super::worldbloomsupportdeckbonuses::WorldbloomsupportdeckbonuseElement;
        let rows: Vec<EventElement> =
            parse(r#"[{"id":1,"name":"e","eventBreakTimeId":1},{"id":2}]"#);
        assert_eq!(rows[0].event_break_time_id, Some(1));
        assert_eq!(rows[1].event_break_time_id, None);
        let rows: Vec<GachaElement> = parse(
            r#"[{"id":1,"gachaType":"normal","isSelectCharacter":false,
                 "gachaCharacterBonusGroupId":1,"rateChoiceGachaWishGroupId":1,
                 "gachaCardRarityRates":[{"id":1,"groupId":1,"cardRarityType":"rarity_4",
                   "lotteryType":"rate_choice_second","rate":3.0}]},
                {"gachaType":"sureturn","id":2}]"#,
        );
        assert_eq!(rows[0].is_select_character, Some(false));
        assert_eq!(
            rows[0].gacha_card_rarity_rates.as_ref().unwrap()[0].lottery_type,
            Some(super::gachas::LotteryType::RateChoiceSecond)
        );
        assert_eq!(rows[1].gacha_type, Some(super::gachas::GachaType::Sureturn));
        assert_eq!(rows[0].gacha_character_bonus_group_id, Some(1));
        assert_eq!(rows[0].rate_choice_gacha_wish_group_id, Some(1));
        assert_eq!(rows[1].is_select_character, None);
        let rows: Vec<LimitedtimemusicElement> =
            parse(r#"[{"id":1,"musicId":1,"collaborationModeId":1}]"#);
        assert_eq!(rows[0].collaboration_mode_id, Some(1));
        let rows: Vec<MusicElement> =
            parse(r#"[{"id":1,"secForMusicScoreMaker":122,"isAvailableForMusicScoreMaker":true}]"#);
        assert_eq!(rows[0].sec_for_music_score_maker, Some(122));
        assert_eq!(rows[0].is_available_for_music_score_maker, Some(true));
        let rows: Vec<MysekaihousingcompetitionElement> =
            parse(r#"[{"id":1,"mysekaiHousingCompetitionReviewRankId":1}]"#);
        assert_eq!(rows[0].mysekai_housing_competition_review_rank_id, Some(1));
        let rows: Vec<PlayerframegroupElement> =
            parse(r#"[{"id":1,"playerFrameType":"single","editCount":0}]"#);
        assert_eq!(rows[0].player_frame_type.as_deref(), Some("single"));
        assert_eq!(rows[0].edit_count, Some(0));
        let rows: Vec<PlayerframeElement> = parse(r#"[{"id":1,"partsCount":0}]"#);
        assert_eq!(rows[0].parts_count, Some(0));
        let rows: Vec<WorldbloomsupportdeckbonuseElement> = parse(
            r#"[{"cardRarityType":"rarity_1",
                 "worldBloomSupportDeckCharacterBonuses":[{"bonusRate":5.5,"id":10101,
                   "worldBloomSupportDeckCharacterType":"specific"}],
                 "worldBloomSupportDeckMasterRankBonuses":[{"bonusRate":0.0,"id":10101,"masterRank":0}],
                 "worldBloomSupportDeckSkillLevelBonuses":[{"bonusRate":0.0,"id":10101,"skillLevel":1}]}]"#,
        );
        let character = rows[0]
            .world_bloom_support_deck_character_bonuses
            .as_ref()
            .unwrap();
        assert_eq!(character[0].bonus_rate, Some(5.5));
        assert_eq!(
            rows[0]
                .world_bloom_support_deck_skill_level_bonuses
                .as_ref()
                .unwrap()[0]
                .skill_level,
            Some(1)
        );
    }
}

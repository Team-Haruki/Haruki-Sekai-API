structures = {
    "actionSets": [
        "id",
        "areaId",
        "actionSetType",
        "isNextGrade",
        "scriptId",
        "scenarioId",
        # "specialSeasonId",
        "characterIds",
        "archiveDisplayType",
        "archivePublishedAt",
        "releaseConditionId",
    ],
    "areaItemLevels": [
        "areaItemId",
        "level",
        "targetUnit",
        "targetCardAttr",
        "targetGameCharacterId",
        "power1BonusRate",
        "power1AllMatchBonusRate",
        "power2BonusRate",
        "power2AllMatchBonusRate",
        "power3BonusRate",
        "power3AllMatchBonusRate",
        "sentence",
    ],
    "bondsHonors": [
        "id", "seq", "bondsGroupId", "gameCharacterUnitId1",
        "gameCharacterUnitId2", "honorRarity", "name", "description",
        ["levels", ["id", "bondsHonorId", "level",
                    "description"]], "configurableUnitVirtualSinger"
    ],
    "cardCostume3ds": ["cardId", "costume3dId"],
    "cardEpisodes": [
        "id", "cardId", "title", "scenarioId", "releaseConditionId",
        "power1BonusFixed", "power2BonusFixed", "power3BonusFixed",
        ["costs", ["resourceId", "resourceType", "resourceLevel",
                   "quantity"]], "cardEpisodePartType"
    ],
    "cards": [
        "id", "seq", "characterId", "cardRarityType",
        "specialTrainingPower1BonusFixed", "specialTrainingPower2BonusFixed",
        "specialTrainingPower3BonusFixed", "attr", "supportUnit", "skillId",
        "cardSkillName", "prefix", "assetbundleName", "gachaPhrase",
        "archiveDisplayType", "archivePublishedAt", "cardParameters",
        [
            "specialTrainingCosts",
            [
                "cardId", "seq",
                [
                    "cost",
                    ("resourceId", "resourceType", "resourceLevel", "quantity")
                ]
            ]
        ], ["masterLessonAchieveResources", ["masterRank",
                                             "resources"]], "releaseAt"
    ],
    "challengeLiveHighScoreRewards":
        ["id", "characterId", "highScore", "resourceBoxId"],
    "challengeLiveStages": ["characterId", "rank", "nextStageChallengePoint"],
    "character3ds": [
        "id", "characterId", "unit", "headCostume3dId", "hairCostume3dId",
        "bodyCostume3dId"
    ],
    "characterArchiveVoices": [
        "id", "groupId", "gameCharacterId", "unit",
        "characterArchiveVoiceType", "displayPhrase", "displayPhrase2",
        "characterArchiveVoiceTagId", "externalId", "assetName", "isNextGrade",
        "displayStartAt"
    ],
    "characterRanks": [
        "id", "characterId", "characterRank", "power1BonusRate",
        "power2BonusRate", "power3BonusRate", "rewardResourceBoxIds",
        ["characterRankAchieveResources", ["resources"]]
    ],
    "cheerfulCarnivalPartyNames": [
        "id", "partyName", "gameCharacterUnitId1", "gameCharacterUnitId2",
        "gameCharacterUnitId3", "gameCharacterUnitId4", "gameCharacterUnitId5"
    ],
    "episodeCharacters":
        ["id", "seq", "character2dId", "storyType", "episodeId"],
    "eventDeckBonuses":
        ["id", "eventId", "gameCharacterUnitId", "cardAttr", "bonusRate"],
    "eventExchangeSummaries": [
        "id", "eventId", "startAt", "endAt",
        [
            "eventExchanges",
            [
                "id", "eventExchangeSummaryId", "unknown1", "seq",
                "resourceBoxId", "exchangeLimit",
                ["eventExchangeCost", ("resourceQuantity")]
            ]
        ]
    ],
    "events": [
        "id", "eventType", "name", "assetbundleName", "bgmAssetbundleName",
        "eventPointAssetbundleName", "eventOnlyComponentDisplayStartAt",
        "startAt", "aggregateAt", "rankingAnnounceAt", "distributionStartAt",
        "eventOnlyComponentDisplayEndAt", "closedAt", "virtualLiveId", "unit",
        [
            "eventRankingRewardRanges",
            ["fromRank", "toRank", ["eventRankingRewards", ["resourceBoxId"]]]
        ]
    ],
    "eventStories": [
        "id", "eventId", "outline", "bannerGameCharacterUnitId",
        "assetbundleName",
        [
            "eventStoryEpisodes",
            [
                "id", "eventStoryId", "unknown1", "episodeNo", "title",
                "assetbundleName", "scenarioId", "releaseConditionId",
                ["episodeRewards", ["unknown1", "unknown2", "resourceBoxId"]]
            ]
        ]
    ],
    "gachaCeilExchangeSummaries": [
        "id", "seq", "assetbundleName", "startAt", "endAt",
        [
            "gachaCeilExchanges",
            [
                "id", "gachaCeilExchangeSummaryId", "seq", "resourceBoxId",
                "exchangeLimit", "gachaCeilExchangeLabelType",
                "substituteLimit",
                [
                    "gachaCeilExchangeCost",
                    ("quantity", "resourceType", "resourceId")
                ],
                [
                    "gachaCeilExchangeSubstituteCosts",
                    ["id", "resourceType", "resourceId", "substituteQuantity"]
                ]
            ]
        ]
    ],
    "gachas": [
        "id", "gachaType", "name", "seq", "assetbundleName", "startAt",
        "endAt", "isShowPeriod", "spinLimit", "gachaCeilItemId",
        "wishSelectCount", "wishFixedSelectCount", "wishLimitedSelectCount",
        "gachaBonusId", "drawableGachaHour",
        ["gachaCardRarityRates", ["cardRarityType", "lotteryType", "rate"]],
        [
            "gachaDetails",
            [
                "id", "gachaId", "cardId", "weight", "fixedBonusWeight",
                "isWish", "gachaDetailWishType"
            ]
        ],
        [
            "gachaBehaviors",
            [
                "id", "gachaId", "gachaBehaviorType", "costResourceType",
                "unknown", "costResourceQuantity", "spinCount", "executeLimit",
                "gachaExtraId", "groupId", "priority", "resourceCategory",
                "gachaSpinnableType"
            ]
        ], ["gachaPickups", ["gachaId", "cardId"]],
        ["gachaInformation", ("gachaId", "summary", "description")]
    ],
    "honors": [
        "id", "seq", "groupId", "honorRarity", "name", "assetbundleName",
        "honorTypeId", "honorMissionType", "startAt",
        [
            "levels",
            [
                "level", "bonus", "description", "startAt", "assetbundleName",
                "honorRarity"
            ]
        ]
    ],
    "liveMissions": [
        "id", "liveMissionPeriodId", "liveMissionType", "requirement",
        ["rewards", ["resourceBoxId"]]
    ],
    "masterLessonRewards": ["cardId", "masterRank", "resourceBoxId", "id"],
    "materialExchangeSummaries": [
        "id", "seq", "exchangeCategory", "materialExchangeType", "name",
        "assetbundleName", "startAt", "endAt", "notificationRemainHour",
        [
            "materialExchanges",
            [
                "id", "materialExchangeSummaryId", "seq", "displayName",
                "isDisplayQuantity", "thumbnailAssetbundleName",
                "resourceBoxId", "refreshCycle", "exchangeLimit", "startAt",
                "unknown",
                [
                    "costs",
                    [
                        "materialExchangeId", "costGroupId", "seq",
                        "resourceId", "quantity"
                    ]
                ]
            ]
        ]
    ],
    "musicDifficulties": [
        "id", "musicId", "musicDifficulty", "playLevel", "releaseConditionId",
        "totalNoteCount"
    ],
    "musics": [
        "id", "seq", "releaseConditionId",
        ["categories", ["musicCategoryName"]], "title", "pronunciation",
        "creatorArtistId", "lyricist", "composer", "arranger", "dancerCount",
        "selfDancerPosition", "assetbundleName", "publishedAt", "releasedAt",
        "fillerSec", "info", "musicCollaborationId", "isNewlyWrittenMusic",
        "isFullLength"
    ],
    "musicTags": ["musicId", "musicTag"],
    "musicVocals": [
        "id", "musicId", "musicVocalType", "seq", "releaseConditionId",
        "caption",
        [
            "characters",
            [
                "id", "musicId", "musicVocalId", "characterType",
                "characterId", "seq"
            ]
        ], "assetbundleName", "specialSeasonId", "archiveDisplayType",
        "archivePublishedAt"
    ],
    "ngWords": ["word"],
    "releaseConditions": [
        "id", "sentence", "releaseConditionType", "releaseConditionTypeId",
        "releaseConditionTypeId2", "releaseConditionTypeLevel",
        "releaseConditionTypeQuantity"
    ],
    "returnMissions": [
        "returnMissionGroupId",
        "id",
        "seq",
        "returnMissionType",
        "requirement",
        "sentence",
        "resourceBoxId",
    ],
    "shopItems": [
        "id", "shopId", "seq", "releaseConditionId", "resourceBoxId",
        [
            "costs",
            [
                "cost",
                ("resourceId", "resourceType", "resourceLevel", "quantity")
            ]
        ], "startAt"
    ],
    "stamps": [
        "id", "stampType", "seq", "name", "assetbundleName",
        "balloonAssetbundleName", "characterId1", "characterId2", "", "", "",
        "gameCharacterUnitId", "archiveDisplayType", "archivePublishedAt",
        "description"
    ],
    "topics": ["id", "topicType", "topicTypeId", "releaseConditionId"],
    "virtualItems": [
        "id", "virtualItemCategory", "seq", "priority", "name",
        "assetbundleName", "costVirtualCoin", "costJewel",
        "effectAssetbundleName", "effectExpressionType", "unknown",
        "gameCharacterUnitId", "unit"
    ],
    "virtualLives": [
        "id", "virtualLiveType", "virtualLivePlatform", "seq", "name",
        "assetbundleName", "screenMvMusicVocalId", "startAt", "endAt",
        "rankingAnnounceAt", "",
        [
            "virtualLiveSetlists",
            [
                "id", "seq", "virtualLiveSetlistType", "assetbundleName",
                "virtualLiveStageId", "musicVocalId", "character3dId1",
                "character3dId2", "character3dId3", "character3dId4",
                "character3dId5", "character3dId6", "virtualLiveId", "musicId"
            ]
        ],
        [
            "virtualLiveBeginnerSchedules",
            [
                "id",
                "virtualLiveId",
                "dayOfWeek",
                "startTime",
                "endTime",
            ]
        ],
        [
            "virtualLiveSchedules",
            [
                "id", "virtualLiveId", "seq", "startAt", "endAt",
                "noticeGroupId"
            ]
        ], ["virtualLiveCharacters", ["gameCharacterUnitId"]],
        ["virtualLiveRewards", ["virtualLiveType", "resourceBoxId"]],
        [
            "virtualLiveWaitingRoom",
            ("id", "lobbyAssetbundleName", "startAt", "endAt")
        ],
        [
            "virtualItems",
            [
                "id", "virtualItemCategory", "seq", "priority", "name",
                "assetbundleName", "costVirtualCoin", "costJewel",
                "effectAssetbundleName", "effectExpressionType", "unknown",
                "gameCharacterUnitId", "unit"
            ]
        ],
        [
            "virtualLiveAppeals",
            [
                "id",
                "virtualLiveId",
                "virtualLiveStageStatus",
                "appealText",
            ]
        ],
        [
            "virtualLiveBackgroundMusics",
            [
                "id",
                "virtualLiveId",
                "backgroundMusicId",
            ]
        ],
        [
            "virtualLiveInformation",
            ("virtualLiveId", "summary", "description")
        ]
    ],
    "wordings": ["wordingKey", "value"]
}


def restore_dict(array_data: list, key_structure: list):
    """
    Original Author: DNARoma
    Original URL: <SECRET>
    convert array to dict with given structure
    :param array_data: array data
    :param key_structure: json structure of the result: dict
    :return: result:  dict
    """
    result = {}

    for i, key in enumerate(key_structure):
        if isinstance(key, str):
            # if key is string, then assign the value to the key
            if array_data[i] is not None:
                result[key] = array_data[i]
        elif isinstance(key, list):
            if isinstance(key[1], list):
                # if key is list and the second element is list, then it is a nested list
                result[key[0]] = [
                    restore_dict(sub_array, key[1])
                    for sub_array in array_data[i] if sub_array is not None
                ]
            elif isinstance(key[1], tuple):
                # if key is list and the second element is tuple, then it is a dict
                result[key[0]] = {
                    key[1][i]: v
                    for i, v in enumerate(array_data[i]) if v is not None
                }

    return result


def nuverse_master_restorer(master_data: dict) -> dict:
    for key, value in master_data.items():
        try:
            id_key = None
            if key == "eventCards":
                id_key = "cardId"
            if key in structures:
                master_data[key] = [restore_dict(file_datum, structures[key]) for file_datum in value]
            if id_key is not None:
                value_ids = {item[id_key] for item in master_data[key]}
                master_data[key] = [x for x in value if x[id_key] not in value_ids] + value
                master_data[key].sort(key=lambda x: x[id_key])
        except Exception as e:
            raise e

    return master_data

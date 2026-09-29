use serde::{Deserialize, Serialize};

pub type Panelmissioncampaign = Vec<PanelmissioncampaignElement>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelmissioncampaignElement {
    pub id: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub bgm_assetbundle_name: Option<String>,

    pub selectable_limit: Option<i64>,

    pub start_at: Option<i64>,

    pub progress_end_at: Option<i64>,

    pub closed_at: Option<i64>,

    pub distribution_end_at: Option<i64>,

    pub information_id: Option<i64>,

    pub panel_mission_selection_type: Option<String>,

    pub panel_mission_sheet_groups: Option<Vec<PanelMissionSheetGroup>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelMissionSheetGroup {
    pub id: Option<i64>,

    pub panel_mission_campaign_id: Option<i64>,

    pub name: Option<String>,

    pub selectable_limit: Option<i64>,

    pub seq: Option<i64>,

    pub panel_mission_sheets: Option<Vec<PanelMissionSheet>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelMissionSheet {
    pub id: Option<i64>,

    pub panel_mission_sheet_group_id: Option<i64>,

    pub name: Option<String>,

    pub assetbundle_name: Option<String>,

    pub seq: Option<i64>,

    pub is_initial_selected: Option<bool>,

    pub panel_missions: Option<Vec<PanelMission>>,

    pub rewards: Option<Vec<PanelMissionReward>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelMission {
    pub id: Option<i64>,

    pub panel_mission_sheet_id: Option<i64>,

    pub name: Option<String>,

    pub description: Option<String>,

    pub seq: Option<i64>,

    pub requirement1: Option<i64>,

    pub requirement2: Option<i64>,

    pub panel_mission_type: Option<String>,

    pub rewards: Option<Vec<PanelMissionReward>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelMissionReward {
    pub id: Option<i64>,

    pub mission_type: Option<String>,

    pub mission_id: Option<i64>,

    pub seq: Option<i64>,

    pub resource_box_id: Option<i64>,
}

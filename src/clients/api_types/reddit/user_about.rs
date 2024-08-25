use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedditUserAbout {
    pub kind: String,
    pub data: RedditUserAboutData,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedditUserAboutData {
    pub name: String,
    #[serde(rename = "is_suspended")]
    pub is_suspended: bool,
    #[serde(rename = "awardee_karma")]
    pub awardee_karma: i64,
    #[serde(rename = "awarder_karma")]
    pub awarder_karma: i64,
    #[serde(rename = "is_blocked")]
    pub is_blocked: bool,
    #[serde(rename = "total_karma")]
    pub total_karma: i64,
}

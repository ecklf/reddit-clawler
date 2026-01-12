use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

pub struct DownloadStats {
    pub downloads_failed: u64,
    pub bytes_downloaded: f64,
    pub files_downloaded: u64,
}

#[derive(Default, Copy, Debug, Clone, PartialEq)]
pub enum FileCacheVersion {
    #[default]
    Latest = 3,
    V2 = 2,
    V1 = 1,
}

impl Serialize for FileCacheVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            FileCacheVersion::V1 => serializer.serialize_i64(1),
            FileCacheVersion::V2 => serializer.serialize_i64(2),
            FileCacheVersion::Latest => serializer.serialize_i64(3),
        }
    }
}

impl<'de> Deserialize<'de> for FileCacheVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = i64::deserialize(deserializer)?;
        match version {
            1 => Ok(FileCacheVersion::V1),
            2 => Ok(FileCacheVersion::V2),
            3 => Ok(FileCacheVersion::Latest),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid version: {}",
                version
            ))),
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartialFileCache {
    pub version: FileCacheVersion,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceStatus {
    #[default]
    Active,
    Deleted,
    Suspended,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LastDownloadStatus {
    #[default]
    Success,
    RateLimit,
    Forbidden,
    Error,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheStatus {
    pub resource: ResourceStatus,
    pub last_download: LastDownloadStatus,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheLatest {
    pub version: FileCacheVersion,
    pub status: FileCacheStatus,
    pub files: Vec<FileCacheItemLatest>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheItemLatest {
    pub id: String,
    pub created_utc: DateTime<Utc>,
    pub title: String,
    pub subreddit: String,
    pub url: String,
    pub success: bool,
    pub index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_gallery: Option<bool>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheItemV1 {
    pub id: String,
    pub created_utc: DateTime<Utc>,
    pub title: String,
    pub subreddit: String,
    pub url: String,
    pub success: bool,
    pub index: Option<usize>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheItemV2 {
    pub id: String,
    pub created_utc: DateTime<Utc>,
    pub title: String,
    pub subreddit: String,
    pub url: String,
    pub success: bool,
    pub index: Option<usize>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheV1 {
    pub version: FileCacheVersion,
    pub files: Vec<FileCacheItemV1>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheV2 {
    pub version: FileCacheVersion,
    pub status: FileCacheStatus,
    pub files: Vec<FileCacheItemV2>,
}

#[derive(Error, Debug)]
pub enum FileCacheError {
    #[error("JSON deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Failed reading cache version")]
    Version,
    #[error("Failed upgrading cache file")]
    Upgrade,
}

pub fn get_cache_from_serde_value(mut value: Value) -> Result<FileCacheLatest, FileCacheError> {
    let PartialFileCache { version } = serde_json::from_value::<PartialFileCache>(value.clone())
        .map_err(FileCacheError::SerdeJson)?;

    match version {
        FileCacheVersion::V1 => {
            value["version"] =
                serde_json::to_value(FileCacheVersion::V2).map_err(FileCacheError::SerdeJson)?;

            value["status"] = serde_json::to_value(FileCacheStatus {
                resource: ResourceStatus::Active,
                last_download: LastDownloadStatus::Success,
            })
            .map_err(FileCacheError::SerdeJson)?;
            get_cache_from_serde_value(value)
        }
        FileCacheVersion::V2 => {
            value["version"] = serde_json::to_value(FileCacheVersion::Latest)
                .map_err(FileCacheError::SerdeJson)?;

            // V2 to V3 migration - is_gallery field is optional, so no changes needed
            get_cache_from_serde_value(value)
        }
        FileCacheVersion::Latest => {
            serde_json::from_value::<FileCacheLatest>(value).map_err(FileCacheError::SerdeJson)
        }
    }
}

impl FromStr for FileCacheLatest {
    type Err = FileCacheError;
    fn from_str(s: &str) -> Result<Self, FileCacheError> {
        let cache_value = serde_json::from_str::<Value>(s).map_err(FileCacheError::SerdeJson)?;
        get_cache_from_serde_value(cache_value)
    }
}

impl Default for DownloadStats {
    fn default() -> Self {
        Self {
            downloads_failed: 0,
            bytes_downloaded: 0.0,
            files_downloaded: 0,
        }
    }
}

pub struct SharedState {
    pub redgifs_token: Option<String>,
    pub file_cache_path: Option<String>,
    pub file_cache: FileCacheLatest,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            redgifs_token: None,
            file_cache_path: None,
            file_cache: FileCacheLatest {
                version: FileCacheVersion::Latest,
                status: FileCacheStatus {
                    resource: ResourceStatus::Active,
                    last_download: LastDownloadStatus::Success,
                },
                files: Vec::new(),
            },
        }
    }
}

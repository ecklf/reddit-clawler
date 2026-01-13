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
    Latest = 4,
    V3 = 3,
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
            FileCacheVersion::V3 => serializer.serialize_i64(3),
            FileCacheVersion::Latest => serializer.serialize_i64(4),
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
            3 => Ok(FileCacheVersion::V3),
            4 => Ok(FileCacheVersion::Latest),
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

// V4 - Latest with snake_case
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileCacheStatus {
    pub resource: ResourceStatus,
    pub last_download: LastDownloadStatus,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileCacheLatest {
    pub version: FileCacheVersion,
    pub status: FileCacheStatus,
    pub files: Vec<FileCacheItemLatest>,
}

impl FileCacheLatest {
    /// Upsert a cache item by id + index + media_id.
    /// If an existing item matches, it updates the item (keeping success=false if the new one is false).
    /// If no match is found, it pushes a new item.
    pub fn upsert_item(&mut self, item: FileCacheItemLatest) {
        // Find existing item by id + index + media_id
        let existing = self.files.iter_mut().find(|f| {
            let id_matches = f.id == item.id;
            let index_matches = f.index == item.index;
            let media_id_matches = match (&item.media_id, &f.media_id) {
                (Some(new_mid), Some(existing_mid)) => new_mid == existing_mid,
                (None, None) => true,
                _ => false,
            };
            id_matches && index_matches && media_id_matches
        });

        match existing {
            Some(existing_item) => {
                // Update existing item
                // Keep success=false if the new item has success=false (as requested)
                if !item.success {
                    existing_item.success = false;
                } else {
                    existing_item.success = item.success;
                }
                existing_item.title = item.title;
                existing_item.url = item.url;
                existing_item.created_utc = item.created_utc;
                existing_item.subreddit = item.subreddit;
                existing_item.is_gallery = item.is_gallery;
                existing_item.media_id = item.media_id;
            }
            None => {
                // No existing item found, push new one
                self.files.push(item);
            }
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
}

// V3 - camelCase with is_gallery
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheStatusV3 {
    pub resource: ResourceStatus,
    pub last_download: LastDownloadStatus,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheV3 {
    pub version: FileCacheVersion,
    pub status: FileCacheStatusV3,
    pub files: Vec<FileCacheItemV3>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheItemV3 {
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

// V2 - camelCase with status
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheStatusV2 {
    pub resource: ResourceStatus,
    pub last_download: LastDownloadStatus,
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
pub struct FileCacheV2 {
    pub version: FileCacheVersion,
    pub status: FileCacheStatusV2,
    pub files: Vec<FileCacheItemV2>,
}

// V1 - camelCase without status
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
pub struct FileCacheV1 {
    pub version: FileCacheVersion,
    pub files: Vec<FileCacheItemV1>,
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

            value["status"] = serde_json::to_value(FileCacheStatusV2 {
                resource: ResourceStatus::Active,
                last_download: LastDownloadStatus::Success,
            })
            .map_err(FileCacheError::SerdeJson)?;
            get_cache_from_serde_value(value)
        }
        FileCacheVersion::V2 => {
            value["version"] = serde_json::to_value(FileCacheVersion::V3)
                .map_err(FileCacheError::SerdeJson)?;

            // V2 to V3 migration - is_gallery field is optional, so no changes needed
            get_cache_from_serde_value(value)
        }
        FileCacheVersion::V3 => {
            // V3 to V4 migration - convert camelCase to snake_case
            // Parse as V3 first
            let v3_cache = serde_json::from_value::<FileCacheV3>(value)
                .map_err(FileCacheError::SerdeJson)?;
            
            // Convert to V4 (snake_case) - field names are same in Rust, just serialization changes
            let v4_cache = FileCacheLatest {
                version: FileCacheVersion::Latest,
                status: FileCacheStatus {
                    resource: v3_cache.status.resource,
                    last_download: v3_cache.status.last_download,
                },
                files: v3_cache
                    .files
                    .into_iter()
                    .map(|item| FileCacheItemLatest {
                        id: item.id,
                        created_utc: item.created_utc,
                        title: item.title,
                        subreddit: item.subreddit,
                        url: item.url,
                        success: item.success,
                        index: item.index,
                        is_gallery: item.is_gallery,
                        media_id: None,
                    })
                    .collect(),
            };
            Ok(v4_cache)
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

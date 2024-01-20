use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub struct DownloadStats {
    pub downloads_failed: u64,
    pub bytes_downloaded: f64,
    pub files_downloaded: u64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCache {
    pub version: i64,
    pub files: Vec<FileCacheItem>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCacheItem {
    pub id: String,
    pub created_utc: DateTime<Utc>,
    pub title: String,
    pub subreddit: String,
    pub url: String,
    pub success: bool,
    pub index: Option<usize>,
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
    pub file_cache: FileCache,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            redgifs_token: None,
            file_cache: FileCache {
                version: 1,
                files: Vec::new(),
            },
        }
    }
}

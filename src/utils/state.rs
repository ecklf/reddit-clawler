pub struct DownloadStats {
    pub downloads_failed: u64,
    pub bytes_downloaded: f64,
    pub files_downloaded: u64,
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
}

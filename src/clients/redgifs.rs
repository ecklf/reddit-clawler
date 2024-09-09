use crate::utils::state::SharedState;
use reqwest::{header::HeaderMap, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedgifsTemporaryTokenResponse {
    pub token: String,
    pub addr: String,
    pub agent: String,
    pub session: String,
    pub rtfm: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedgifsGifResponse {
    pub gif: RedgifsGif,
    // pub user: Value,
    // pub niches: Vec<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedgifsGif {
    pub id: String,
    // #[serde(rename = "client_id")]
    // pub client_id: Option<String>,
    pub create_date: i64,
    // pub has_audio: bool,
    // pub width: i64,
    // pub height: i64,
    // pub hls: bool,
    // pub likes: i64,
    // pub niches: Vec<Value>,
    // pub tags: Vec<String>,
    // pub verified: bool,
    // pub views: Value,
    // pub description: String,
    // pub duration: f64,
    // pub published: bool,
    pub urls: RedgifsUrls,
    // pub user_name: String,
    // #[serde(rename = "type")]
    // pub type_field: i64,
    // pub avg_color: String,
    // pub gallery: Value,
    // pub hide_home: bool,
    // pub hide_trending: bool,
    // pub sexuality: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedgifsUrls {
    // pub vthumbnail: String,
    // pub thumbnail: String,
    // pub poster: String,
    pub hd: String,
    pub sd: String,
}

pub enum RedgifsQuality {
    SD,
    HD,
}

#[derive(Error, Debug)]
pub enum RedgifsClientError {
    #[error("ReqwestMiddleware error: {0}")]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("JSON deserialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("ID extraction failed")]
    ExtractionFailed,
}

// lazy_static! {
//     static ref HEADERS: HeaderMap = {
//         let mut map: HeaderMap = reqwest::header::HeaderMap::new();
//         map.insert(
//             reqwest::header::USER_AGENT,
//             reqwest::header::HeaderValue::from_static("Reddit-User-Analysis"),
//         ).unwrap()
//
//         map
//     };
// }

fn get_header_map() -> HeaderMap {
    let mut map: HeaderMap = reqwest::header::HeaderMap::new();
    map.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("Reddit-User-Analysis"),
    );

    map
}

/// https://github.com/Redgifs/api/wiki/Temporary-tokens
async fn get_temporary_token(
    client: &reqwest_middleware::ClientWithMiddleware,
) -> Result<RedgifsTemporaryTokenResponse, RedgifsClientError> {
    client
        .get("https://api.redgifs.com/v2/auth/temporary")
        .headers(get_header_map())
        .send()
        .await
        .map_err(RedgifsClientError::ReqwestMiddleware)?
        .json::<RedgifsTemporaryTokenResponse>()
        .await
        .map_err(RedgifsClientError::Reqwest)
}

pub async fn download_redgifs_media(
    client: &reqwest_middleware::ClientWithMiddleware,
    shared_state: &Arc<Mutex<SharedState>>,
    url: &str,
    gif_quality: RedgifsQuality,
) -> Result<Response, RedgifsClientError> {
    let mut state = shared_state.lock().await;

    let token = match &state.redgifs_token {
        Some(t) => t.clone(),
        None => {
            let res = get_temporary_token(client).await?;
            state.redgifs_token = Some(res.token.clone());
            res.token
        }
    };

    let post_id = match url {
        _ if url.contains("redgifs.com/i/") => url
            .split("/i/")
            .last()
            .ok_or(RedgifsClientError::ExtractionFailed)?
            .split('.')
            .next()
            .ok_or(RedgifsClientError::ExtractionFailed)?,
        _ if url.contains("redgifs.com/watch/") => url
            .split("/watch/")
            .last()
            .ok_or(RedgifsClientError::ExtractionFailed)?,
        _ if url.contains("redgifs.com/ifr/") => url
            .split("/ifr/")
            .last()
            .ok_or(RedgifsClientError::ExtractionFailed)?,
        _ => return Err(RedgifsClientError::ExtractionFailed),
    };

    let res: RedgifsGifResponse = client
        .get(format!("https://api.redgifs.com/v2/gifs/{}", post_id))
        .headers(get_header_map())
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(RedgifsClientError::ReqwestMiddleware)?
        .json()
        .await
        .map_err(RedgifsClientError::Reqwest)?;

    let dl_url = match gif_quality {
        RedgifsQuality::SD => res.gif.urls.sd,
        RedgifsQuality::HD => res.gif.urls.hd,
    };

    client
        .get(dl_url)
        .headers(get_header_map())
        .send()
        .await
        .map_err(RedgifsClientError::ReqwestMiddleware)
}

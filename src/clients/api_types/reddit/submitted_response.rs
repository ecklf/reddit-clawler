use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{self, Value};
use std::collections::HashMap;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedditSubmittedResponse {
    pub kind: String,
    pub data: Data,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    pub after: Option<String>,
    // pub dist: i64,
    // pub modhash: String,
    // #[serde(rename = "geo_filter")]
    // pub geo_filter: String,
    pub children: Vec<RedditSubmittedChild>,
    pub before: Option<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedditSubmittedChild {
    pub kind: String,
    pub data: RedditSubmittedChildData,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedditSubmittedChildData {
    pub subreddit: String,
    pub title: String,
    // #[serde(rename = "media_embed")]
    // pub media_embed: MediaEmbed,
    // #[serde(rename = "secure_media")]
    // pub secure_media: Option<SecureMedia>,
    #[serde(rename = "is_reddit_media_domain")]
    pub is_reddit_media_domain: bool,
    // #[serde(rename = "is_meta")]
    // pub is_meta: bool,
    // pub category: Value,
    // pub thumbnail: String,
    // pub created: f64,
    // pub url_overridden_by_dest: Option<String>,
    // #[serde(rename = "over_18")]
    // pub over_18: bool,
    // pub preview: Option<Preview>,
    #[serde(rename = "media_only")]
    pub media_only: bool,
    // #[serde(rename = "subreddit_id")]
    // pub subreddit_id: String,
    pub ups: i64,
    pub id: String,
    pub author: String,
    pub url: String,
    #[serde(rename = "created_utc")]
    #[serde(deserialize_with = "shitty_reddit_datetime_utc")]
    pub created_utc: DateTime<Utc>,
    pub media: Option<Media>,
    #[serde(rename = "is_video")]
    pub is_video: Option<bool>,
    #[serde(rename = "is_gallery")]
    pub is_gallery: Option<bool>,
    #[serde(rename = "media_metadata")]
    pub media_metadata: Option<HashMap<String, MediaMetadataValue>>,
    #[serde(rename = "gallery_data")]
    pub gallery_data: Option<GalleryData>,
}

fn shitty_reddit_datetime_utc<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let timestamp: f64 = Deserialize::deserialize(deserializer)?;
    // Convert the floating-point timestamp to i64 and then to DateTime<Utc>
    let utc_timestamp_seconds = (timestamp * 1000.0).round() as i64;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(
        chrono::NaiveDateTime::from_timestamp_opt(utc_timestamp_seconds / 1000, 0).unwrap(),
        Utc,
    ))
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkFlairRichtext {
    pub e: String,
    pub t: Option<String>,
    pub a: Option<String>,
    pub u: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaEmbed {
    pub content: Option<String>,
    pub width: Option<i64>,
    pub scrolling: Option<bool>,
    pub height: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecureMedia {
    #[serde(rename = "type")]
    pub type_field: Option<String>,
    pub oembed: Option<Oembed>,
    #[serde(rename = "reddit_video")]
    pub reddit_video: Option<RedditVideo>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedditVideo {
    #[serde(rename = "bitrate_kbps")]
    pub bitrate_kbps: i64,
    #[serde(rename = "fallback_url")]
    pub fallback_url: String,
    pub height: i64,
    pub width: i64,
    #[serde(rename = "scrubber_media_url")]
    pub scrubber_media_url: String,
    #[serde(rename = "dash_url")]
    pub dash_url: String,
    pub duration: i64,
    #[serde(rename = "hls_url")]
    pub hls_url: String,
    #[serde(rename = "is_gif")]
    pub is_gif: bool,
    #[serde(rename = "transcoding_status")]
    pub transcoding_status: String,
    #[serde(rename = "has_audio")]
    pub has_audio: Option<bool>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Oembed {
    #[serde(rename = "provider_url")]
    pub provider_url: String,
    pub version: String,
    pub title: String,
    #[serde(rename = "type")]
    pub type_field: Option<String>,
    #[serde(rename = "thumbnail_width")]
    pub thumbnail_width: i64,
    pub height: i64,
    pub width: i64,
    pub html: String,
    #[serde(rename = "author_name")]
    pub author_name: Option<String>,
    #[serde(rename = "provider_name")]
    pub provider_name: String,
    #[serde(rename = "thumbnail_url")]
    pub thumbnail_url: String,
    #[serde(rename = "thumbnail_height")]
    pub thumbnail_height: i64,
    #[serde(rename = "author_url")]
    pub author_url: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecureMediaEmbed {
    pub content: Option<String>,
    pub width: Option<i64>,
    pub scrolling: Option<bool>,
    #[serde(rename = "media_domain_url")]
    pub media_domain_url: Option<String>,
    pub height: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorFlairRichtext {
    pub e: String,
    pub t: Option<String>,
    pub a: Option<String>,
    pub u: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gildings {}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub images: Vec<Image>,
    pub enabled: bool,
    #[serde(rename = "reddit_video_preview")]
    pub reddit_video_preview: Option<RedditVideoPreview>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub source: Source,
    pub resolutions: Vec<Resolution>,
    pub variants: Variants,
    pub id: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub url: String,
    pub width: i64,
    pub height: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub url: String,
    pub width: i64,
    pub height: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variants {
    pub obfuscated: Option<Obfuscated>,
    pub nsfw: Option<Nsfw>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Obfuscated {
    pub source: Source,
    pub resolutions: Vec<Resolution>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Nsfw {
    pub source: Source,
    pub resolutions: Vec<Resolution>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedditVideoPreview {
    #[serde(rename = "bitrate_kbps")]
    pub bitrate_kbps: i64,
    #[serde(rename = "fallback_url")]
    pub fallback_url: String,
    pub height: i64,
    pub width: i64,
    #[serde(rename = "scrubber_media_url")]
    pub scrubber_media_url: String,
    #[serde(rename = "dash_url")]
    pub dash_url: String,
    pub duration: i64,
    #[serde(rename = "hls_url")]
    pub hls_url: String,
    #[serde(rename = "is_gif")]
    pub is_gif: bool,
    #[serde(rename = "transcoding_status")]
    pub transcoding_status: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    #[serde(rename = "type")]
    pub type_field: Option<String>,
    pub oembed: Option<Oembed>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadataValue {
    pub status: String,
    // pub e: String,
    // pub m: String,
    // pub o: Option<Vec<O>>,
    // pub p: Vec<P>,
    pub s: Option<S>,
    pub id: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct O {
    pub y: i64,
    pub x: i64,
    pub u: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P {
    pub y: i64,
    pub x: i64,
    pub u: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct S {
    pub y: i64,
    pub x: i64,
    pub u: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryData {
    pub items: Vec<Item>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    #[serde(rename = "media_id")]
    pub media_id: String,
    pub id: i64,
}

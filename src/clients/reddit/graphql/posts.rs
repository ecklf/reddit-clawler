//! Converts raw post objects, not feed edges or GraphQL response envelopes.
//!
//! Schema assumptions: SubredditPost/ProfilePost/DeletedSubredditPost use `id`,
//! `createdAt`, `postTitle` (or `title`), `subreddit.name` (or `profile.name`),
//! `authorInfo.name`, optional integer `score`, and `url`. Dates are ISO-8601,
//! including Reddit's fractional-seconds/+0000 spelling. Post IDs strip `t3_`
//! for legacy cache compatibility; media IDs remain verbatim. Deleted tombstones
//! without either title field are skipped, while missing deleted authors/URLs
//! are tolerated. Media uses `streaming.hlsUrl`,
//! `animated.mp4_source`/`mp4_xxxlarge`, or `still.<size>.url`. Still-only media
//! requires a non-self post URL on i.redd.it or preview.redd.it; otherwise it is
//! only a link preview and must not become downloadable native media.
//! Gallery order is `gallery.items` order; each item's
//! string `id` (falling back to `media.id`) is its media ID, not a URL-derived ID.
//! Gallery assets have a direct `url` or named size objects with `url` and
//! optional `dimensions.{width,height}`. Other node types (including ads and
//! unavailable nodes) are intentionally omitted. No reference implementation
//! code is used here; the existing typed response is the parser boundary.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::clients::{
    api_types::reddit::submitted_response::{
        Data, GalleryData, Image, Item, Media, MediaMetadataValue, Mp4, Preview,
        RedditSubmittedChild, RedditSubmittedChildData, RedditSubmittedResponse, RedditVideo,
        Source, Variants, S,
    },
    reddit::RedditProviderError,
};

const STILL_SIZES: &[&str] = &[
    "source",
    "xxxlarge",
    "xxlarge",
    "xlargePlus720",
    "xlarge",
    "midlarge480",
    "large",
    "medium",
    "small",
];
const ASSET_SIZES: &[&str] = &["xxxlarge", "xxlarge", "xlarge", "large", "medium", "small"];

type ConversionResult<T> = Result<T, RedditProviderError>;

pub(super) fn listing(
    posts: &[Value],
    after: Option<String>,
) -> Result<RedditSubmittedResponse, RedditProviderError> {
    let mut children = Vec::with_capacity(posts.len());
    for post in posts {
        if post.is_null() {
            continue;
        }
        if !post.is_object() {
            return Err(RedditProviderError::GraphQl("post node is not an object"));
        }
        let kind = required_string(&post["__typename"], "post has no __typename")?;
        match kind {
            "SubredditPost" | "ProfilePost" | "DeletedSubredditPost" => {}
            // This also excludes unavailable posts, ad nodes and future union members.
            _ => continue,
        }
        if post.get("adPayload").is_some_and(|value| !value.is_null()) {
            continue;
        }
        let deleted = kind == "DeletedSubredditPost";
        // Deleted union members may be only tombstones, not complete posts.
        // Omit those explicitly rather than aborting an otherwise valid page.
        if deleted && post["postTitle"].is_null() && post["title"].is_null() {
            continue;
        }
        children.push(RedditSubmittedChild {
            kind: Some("t3".into()),
            data: convert(post, deleted)?,
        });
    }
    Ok(RedditSubmittedResponse {
        kind: Some("Listing".into()),
        data: Data {
            after,
            children,
            ..Default::default()
        },
    })
}

fn required_string<'a>(value: &'a Value, message: &'static str) -> ConversionResult<&'a str> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or(RedditProviderError::GraphQl(message))
}

fn post_id(value: &Value) -> ConversionResult<String> {
    let id = required_string(value, "post has no id")?;
    let bare = id.strip_prefix("t3_").unwrap_or(id);
    if bare.is_empty() || !bare.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(RedditProviderError::GraphQl("invalid post id"));
    }
    Ok(bare.to_owned())
}

fn timestamp(value: &Value) -> ConversionResult<DateTime<Utc>> {
    let value = required_string(value, "post has no createdAt")?;
    DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .map(|date| date.with_timezone(&Utc))
        .map_err(|_| RedditProviderError::GraphQl("invalid post createdAt"))
}

fn optional_object<'a>(value: &'a Value, field: &str) -> ConversionResult<Option<&'a Value>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value)),
        _ => Err(RedditProviderError::GraphQl("invalid media object")),
    }
}

fn media_url(value: &Value) -> ConversionResult<String> {
    let raw = required_string(value, "media has no URL")?;
    let parsed =
        url::Url::parse(raw).map_err(|_| RedditProviderError::GraphQl("invalid media URL"))?;
    if !matches!(parsed.scheme(), "https" | "http") || parsed.host_str().is_none() {
        return Err(RedditProviderError::GraphQl("invalid media URL"));
    }
    Ok(raw.to_owned())
}

fn dimension(value: Option<&Value>) -> ConversionResult<i64> {
    match value {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_i64()
            .filter(|value| *value > 0)
            .ok_or(RedditProviderError::GraphQl("invalid media dimensions")),
    }
}

fn source(value: &Value) -> ConversionResult<Source> {
    let dimensions = optional_object(value, "dimensions")?;
    Ok(Source {
        url: media_url(&value["url"])?,
        width: dimension(dimensions.and_then(|d| d.get("width")))?,
        height: dimension(dimensions.and_then(|d| d.get("height")))?,
    })
}

fn sized_source(value: &Value, sizes: &[&str]) -> ConversionResult<Source> {
    for size in sizes {
        if let Some(candidate) = value.get(*size).filter(|value| !value.is_null()) {
            return source(candidate);
        }
    }
    Err(RedditProviderError::GraphQl("media has no usable source"))
}

// Route known external media before looking at Reddit previews or silent copies.
fn external_media(url: &str) -> Option<Option<&'static str>> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if on("youtube.com") || on("youtu.be") {
        Some(Some("youtube.com"))
    } else if on("redgifs.com") || on("imgur.com") {
        Some(None)
    } else {
        None
    }
}

fn convert(post: &Value, deleted: bool) -> ConversionResult<RedditSubmittedChildData> {
    let title = post
        .get("postTitle")
        .filter(|value| !value.is_null())
        .unwrap_or(&post["title"])
        .as_str()
        .ok_or(RedditProviderError::GraphQl("post has no title"))?;
    let community = post
        .get("subreddit")
        .filter(|value| !value.is_null())
        .unwrap_or(&post["profile"]);
    // Account deletion can remove authorInfo without deleting the post or its
    // media; this is independent of the post union member.
    let author = post["authorInfo"]["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .unwrap_or("[deleted]");
    let mut data = RedditSubmittedChildData {
        id: post_id(&post["id"])?,
        created_utc: timestamp(&post["createdAt"])?,
        title: title.to_owned(),
        subreddit: required_string(&community["name"], "post has no community")?.to_owned(),
        author: author.to_owned(),
        ups: match post.get("score") {
            None | Some(Value::Null) => 0,
            Some(score) => score
                .as_i64()
                .ok_or(RedditProviderError::GraphQl("invalid post score"))?,
        },
        is_video: Some(false),
        is_gallery: Some(false),
        ..Default::default()
    };
    match post.get("url") {
        None | Some(Value::Null) if deleted || post["isSelfPost"] == true => {}
        Some(value) => data.url = media_url(value)?,
        _ => return Err(RedditProviderError::GraphQl("post has no URL")),
    }
    if let Some(provider) = external_media(&data.url) {
        data.media = provider.map(|provider| Media {
            type_field: Some(provider.into()),
            ..Default::default()
        });
        return Ok(data);
    }
    if let Some(gallery) = optional_object(post, "gallery")? {
        convert_gallery(gallery, &mut data)?;
        return Ok(data);
    }
    if let Some(media) = optional_object(post, "media")? {
        if let Some(streaming) = optional_object(media, "streaming")? {
            data.is_reddit_media_domain = true;
            data.is_video = Some(true);
            data.media = Some(Media {
                reddit_video: Some(RedditVideo {
                    hls_url: Some(media_url(&streaming["hlsUrl"])?),
                    ..Default::default()
                }),
                ..Default::default()
            });
        } else if let Some(animated) = optional_object(media, "animated")? {
            let mp4 = sized_source(animated, &["mp4_source", "mp4_xxxlarge"])?;
            data.is_reddit_media_domain = true;
            data.preview = Some(Preview {
                enabled: true,
                images: vec![Image {
                    variants: Variants {
                        mp4: Some(Mp4 {
                            source: mp4,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            });
        } else if let Some(still) = optional_object(media, "still")? {
            // Link and self posts can carry Reddit-hosted thumbnail previews.
            // The destination, not the preview source, determines native stills.
            let native_still = url::Url::parse(&data.url)
                .is_ok_and(|url| matches!(url.host_str(), Some("i.redd.it" | "preview.redd.it")));
            if post["isSelfPost"] == true || !native_still {
                return Ok(data);
            }
            let still = sized_source(still, STILL_SIZES)?;
            data.url = still.url.clone();
            data.is_reddit_media_domain = true;
            data.preview = Some(Preview {
                enabled: true,
                images: vec![Image {
                    source: still,
                    ..Default::default()
                }],
                ..Default::default()
            });
        } else if post["isSelfPost"] != true
            && url::Url::parse(&data.url).is_ok_and(|url| {
                matches!(
                    url.host_str(),
                    Some("i.redd.it" | "preview.redd.it" | "v.redd.it")
                )
            })
        {
            return Err(RedditProviderError::GraphQl(
                "unsupported or empty native post media",
            ));
        }
        // Text/link posts can have a media container whose downloadable slots
        // are all null. Retain their metadata, with no parser download output.
    }
    // A genuine text/ordinary link post can legitimately have no parser output.
    Ok(data)
}

fn convert_gallery(gallery: &Value, data: &mut RedditSubmittedChildData) -> ConversionResult<()> {
    let items = gallery["items"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or(RedditProviderError::GraphQl("gallery has no items"))?;
    let mut ordered = Vec::with_capacity(items.len());
    let mut metadata = HashMap::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let media = optional_object(item, "media")?
            .ok_or(RedditProviderError::GraphQl("gallery item has no media"))?;
        let id = required_string(
            item.get("id")
                .filter(|id| id.is_string())
                .unwrap_or(&media["id"]),
            "gallery item has no media id",
        )?;
        if id.trim() != id
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(RedditProviderError::GraphQl("invalid gallery media id"));
        }
        let asset = if media.get("url").is_some_and(|url| !url.is_null()) {
            Source {
                url: media_url(&media["url"])?,
                width: dimension(media.get("width"))?,
                height: dimension(media.get("height"))?,
            }
        } else {
            sized_source(media, ASSET_SIZES)?
        };
        if metadata
            .insert(
                id.to_owned(),
                MediaMetadataValue {
                    status: "valid".into(),
                    id: Some(id.to_owned()),
                    s: Some(S {
                        u: Some(asset.url),
                        x: asset.width,
                        y: asset.height,
                        ..Default::default()
                    }),
                },
            )
            .is_some()
        {
            return Err(RedditProviderError::GraphQl("duplicate gallery media id"));
        }
        ordered.push(Item {
            media_id: id.to_owned(),
            id: index as i64,
        });
    }
    data.is_gallery = Some(true);
    data.gallery_data = Some(GalleryData { items: ordered });
    data.media_metadata = Some(metadata);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reddit_parser::{RedditCrawlerPost, RedditMediaProviderType, RedditPostParser};
    use serde_json::json;

    fn post() -> Value {
        json!({
            "__typename": "SubredditPost",
            "id": "t3_AbC123",
            "createdAt": "2026-10-04T00:26:11.575000+0000",
            "postTitle": "Original title",
            "subreddit": {"name": "pics"},
            "authorInfo": {"name": "OriginalAuthor"},
            "score": 42,
            "url": "https://www.reddit.com/r/pics/comments/AbC123/"
        })
    }

    fn parsed(post: Value) -> Vec<RedditCrawlerPost> {
        RedditPostParser::default().parse(&listing(&[post], None).unwrap())
    }

    fn assert_invalid(post: Value) {
        assert!(matches!(
            listing(&[post], None),
            Err(RedditProviderError::GraphQl(_))
        ));
    }

    #[test]
    fn single_image_preserves_identity_time_and_cursor() {
        let mut post = post();
        post["url"] = json!("https://i.redd.it/Original.jpg");
        post["media"] = json!({"still": {
            "small": {"url": "https://preview.redd.it/small.jpg"},
            "source": {"url": "https://i.redd.it/Original.jpg", "dimensions": {"width": 1200, "height": 800}}
        }});
        let response = listing(&[post], Some("opaque-cursor".into())).unwrap();
        assert_eq!(response.data.after.as_deref(), Some("opaque-cursor"));
        let output = RedditPostParser::default().parse(&response);
        assert_eq!(output.len(), 1);
        let output = &output[0];
        assert_eq!(output.id, "AbC123");
        assert_eq!(output.author, "OriginalAuthor");
        assert_eq!(output.subreddit, "pics");
        assert_eq!(output.title, "Original title");
        assert_eq!(output.upvotes, 42);
        assert_eq!(
            output.created_utc.to_rfc3339(),
            "2026-10-04T00:26:11.575+00:00"
        );
        assert_eq!(output.provider, RedditMediaProviderType::RedditImage);
        assert_eq!(output.url, "https://i.redd.it/Original.jpg");
    }

    #[test]
    fn post_ids_match_legacy_bare_ids_without_changing_cursor_or_input() {
        let fullname = post();
        let mut bare = fullname.clone();
        bare["id"] = json!("AbC123");
        let cursor = Some("t3_Next456".to_owned());
        let response = listing(&[fullname.clone()], cursor.clone()).unwrap();
        assert_eq!(response, listing(&[bare], cursor).unwrap());
        assert_eq!(response.data.children[0].data.id, "AbC123");
        assert_eq!(response.data.after.as_deref(), Some("t3_Next456"));
        assert_eq!(fullname["id"], "t3_AbC123");
    }

    #[test]
    fn ordinary_link_and_self_thumbnails_do_not_become_native_media() {
        for (url, is_self) in [
            (Some("https://example.com/article"), false),
            (
                Some("https://www.reddit.com/r/pics/comments/another/"),
                false,
            ),
            (Some("https://i.redd.it.example.com/article"), false),
            (Some("https://www.reddit.com/r/pics/comments/AbC123/"), true),
            (Some("https://i.redd.it/linked-in-self-text.jpg"), true),
            (None, true),
        ] {
            let mut post = post();
            post["isSelfPost"] = json!(is_self);
            if let Some(url) = url {
                post["url"] = json!(url);
            } else {
                post.as_object_mut().unwrap().remove("url");
            }
            let without_preview = listing(&[post.clone()], None).unwrap();
            post["media"] = json!({"still": {
                "source": {"url": "https://preview.redd.it/article-thumbnail.jpg"}
            }});
            let response = listing(&[post], None).unwrap();
            assert_eq!(response, without_preview, "{url:?}, self={is_self}");
            let data = &response.data.children[0].data;
            assert_eq!(data.url, url.unwrap_or(""));
            assert!(!data.is_reddit_media_domain);
            assert!(data.preview.is_none());
            assert!(RedditPostParser::default().parse(&response).is_empty());
        }
    }

    #[test]
    fn native_still_destinations_keep_size_fallback() {
        for url in [
            "https://i.redd.it/Original.jpg",
            "https://preview.redd.it/Original.jpg",
        ] {
            let mut post = post();
            post["url"] = json!(url);
            post["isSelfPost"] = json!(false);
            post["media"] = json!({"still": {
                "source": null,
                "xxxlarge": {"url": "https://preview.redd.it/full.jpg"},
                "small": {"url": "https://preview.redd.it/thumbnail.jpg"}
            }});
            let output = parsed(post);
            assert_eq!(output.len(), 1);
            assert_eq!(output[0].id, "AbC123");
            assert_eq!(output[0].provider, RedditMediaProviderType::RedditImage);
            assert_eq!(output[0].url, "https://preview.redd.it/full.jpg");
        }
    }

    #[test]
    fn reddit_video_uses_hls_not_thumbnail() {
        let mut post = post();
        post["media"] = json!({
            "streaming": {"hlsUrl": "https://v.redd.it/abc/HLSPlaylist.m3u8?a=1&b=2"},
            "still": {"source": {"url": "https://i.redd.it/thumbnail.jpg"}}
        });
        let output = parsed(post);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].provider, RedditMediaProviderType::RedditVideo);
        assert_eq!(output[0].extension, "mp4");
        assert_eq!(
            output[0].url,
            "https://v.redd.it/abc/HLSPlaylist.m3u8?a=1&b=2"
        );
    }

    #[test]
    fn animated_mp4_uses_existing_preview_variant() {
        for field in ["mp4_source", "mp4_xxxlarge"] {
            let mut post = post();
            post["media"] =
                json!({"animated": {(field): {"url": "https://preview.redd.it/animated.mp4"}}});
            let output = parsed(post);
            assert_eq!(output.len(), 1);
            // This is the existing parser's classification of MP4 preview variants.
            assert_eq!(output[0].provider, RedditMediaProviderType::RedditImage);
            assert_eq!(output[0].extension, "mp4");
            assert_eq!(output[0].url, "https://preview.redd.it/animated.mp4");
        }
    }

    #[test]
    fn gallery_keeps_item_order_and_raw_media_ids() {
        let mut post = post();
        post["gallery"] = json!({"items": [
            {"id": "t3_Z9", "media": {"url": "https://i.redd.it/first.jpg"}},
            {"media": {"id": "A1", "xxxlarge": {"url": "https://i.redd.it/second.png"}}}
        ]});
        let response = listing(&[post], None).unwrap();
        let output = RedditPostParser::default().parse(&response);
        assert_eq!(output.len(), 2);
        for (index, id) in ["t3_Z9", "A1"].iter().enumerate() {
            assert_eq!(output[index].id, "AbC123");
            assert_eq!(output[index].media_id.as_deref(), Some(*id));
            assert_eq!(output[index].index, Some(index));
            assert_eq!(output[index].is_gallery, Some(true));
            assert_eq!(
                output[index].provider,
                RedditMediaProviderType::RedditGalleryImage
            );
        }
        assert_eq!(output[0].url, "https://i.redd.it/first.jpg");
        assert_eq!(output[1].url, "https://i.redd.it/second.png");
    }

    #[test]
    fn external_links_win_over_reddit_previews_and_streams() {
        for (url, provider) in [
            (
                "https://www.redgifs.com/watch/Original",
                RedditMediaProviderType::RedgifsVideo,
            ),
            (
                "https://www.redgifs.com/ifr/Original",
                RedditMediaProviderType::RedgifsVideo,
            ),
            (
                "https://www.redgifs.com/i/Original",
                RedditMediaProviderType::RedgifsImage,
            ),
            (
                "https://i.imgur.com/Original.gifv",
                RedditMediaProviderType::ImgurImage,
            ),
            (
                "https://www.youtube.com/watch?v=Original",
                RedditMediaProviderType::YoutubeVideo,
            ),
            (
                "https://youtu.be/Original",
                RedditMediaProviderType::YoutubeVideo,
            ),
        ] {
            let mut post = post();
            post["url"] = json!(url);
            post["media"] = json!({
                "streaming": {"hlsUrl": "https://v.redd.it/silent/HLSPlaylist.m3u8"},
                "still": {"source": {"url": "https://i.redd.it/thumbnail.jpg"}}
            });
            let output = parsed(post);
            assert_eq!(output.len(), 1, "{url}");
            assert_eq!(output[0].provider, provider, "{url}");
            assert_eq!(output[0].url, url);
        }
    }

    #[test]
    fn dates_require_valid_iso_timestamps_and_keep_subseconds() {
        for date in [
            "not-a-date",
            "2026-02-30T00:00:00Z",
            "",
            "999999-01-01T00:00:00Z",
        ] {
            let mut post = post();
            post["createdAt"] = json!(date);
            assert_invalid(post);
        }
        let mut post = post();
        post["createdAt"] = json!("2026-10-04T02:26:11.123456+02:00");
        let response = listing(&[post], None).unwrap();
        assert_eq!(
            response.data.children[0].data.created_utc.to_rfc3339(),
            "2026-10-04T00:26:11.123456+00:00"
        );
    }

    #[test]
    fn missing_required_fields_and_invalid_ids_fail_the_whole_listing() {
        for field in [
            "id",
            "createdAt",
            "postTitle",
            "subreddit",
            "url",
            "__typename",
        ] {
            let mut invalid = post();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(listing(&[post(), invalid], None).is_err(), "{field}");
        }
        for id in ["", "t3_", "t1_abc", " abc", "abc/def"] {
            let mut post = post();
            post["id"] = json!(id);
            assert_invalid(post);
        }
        let mut post = post();
        post["score"] = json!("42");
        assert_invalid(post);
    }

    #[test]
    fn text_and_link_posts_with_null_media_slots_keep_metadata_without_downloads() {
        for is_self in [true, false] {
            let mut value = post();
            value["isSelfPost"] = json!(is_self);
            value["media"] = json!({
                "__typename": "PostMedia", "animated": null, "download": null,
                "obfuscated_still": null, "packagedMedia": null, "still": null,
                "streaming": null, "typeHint": null, "video": null
            });
            if !is_self {
                value["url"] = json!("https://example.com/article");
            }
            let response = listing(&[value], Some("next".into())).unwrap();
            assert_eq!(response.data.children.len(), 1);
            assert_eq!(response.data.after.as_deref(), Some("next"));
            assert!(RedditPostParser::default().parse(&response).is_empty());
        }
    }

    #[test]
    fn malformed_supported_media_is_not_an_empty_success() {
        for media in [
            json!({}),
            json!({"streaming": {}}),
            json!({"streaming": {"hlsUrl": "not a URL"}}),
            json!({"animated": {}}),
            json!({"still": {}}),
            json!({"still": {"source": {}}}),
            json!({"still": []}),
        ] {
            let mut post = post();
            post["url"] = json!("https://i.redd.it/Original.jpg");
            post["media"] = media;
            assert_invalid(post);
        }
        for gallery in [
            json!({}),
            json!({"items": []}),
            json!({"items": [{"id": "abc"}]}),
            json!({"items": [{"media": {"url": "https://i.redd.it/a.jpg"}}]}),
            json!({"items": [{"id": "abc", "media": {}}]}),
            json!({"items": [
                {"id": "abc", "media": {"url": "https://i.redd.it/a.jpg"}},
                {"id": "abc", "media": {"url": "https://i.redd.it/b.jpg"}}
            ]}),
        ] {
            let mut post = post();
            post["gallery"] = gallery;
            assert_invalid(post);
        }
    }

    #[test]
    fn unsupported_unavailable_and_ad_nodes_are_explicitly_skipped() {
        let mut ad = post();
        ad["adPayload"] = json!({"id": "ad"});
        let response = listing(
            &[
                Value::Null,
                json!({"__typename": "UnavailablePost"}),
                json!({"__typename": "AdPost"}),
                json!({"__typename": "FutureUnionMember"}),
                ad,
            ],
            Some("next".into()),
        )
        .unwrap();
        assert!(response.data.children.is_empty());
        assert!(RedditPostParser::default().parse(&response).is_empty());
        assert_eq!(response.data.after.as_deref(), Some("next"));
        assert_invalid(json!(17));
    }

    #[test]
    fn deleted_tombstones_without_titles_are_skipped_without_losing_other_posts() {
        let mut image = post();
        image["url"] = json!("https://i.redd.it/Original.jpg");
        image["media"] = json!({"still": {"source": {"url": "https://i.redd.it/Original.jpg"}}});
        for title in [None, Some(Value::Null)] {
            let mut deleted = json!({
                "__typename": "DeletedSubredditPost",
                "id": "t3_Deleted123",
                "createdAt": "2026-10-04T00:26:11Z",
                "subreddit": {"name": "pics"}
            });
            if let Some(title) = title {
                deleted["postTitle"] = title;
            }
            let response = listing(&[deleted, image.clone()], Some("next".into())).unwrap();
            assert_eq!(response.data.children.len(), 1);
            assert_eq!(response.data.after.as_deref(), Some("next"));
            let output = RedditPostParser::default().parse(&response);
            assert_eq!(output.len(), 1);
            assert_eq!(output[0].id, "AbC123");
        }
    }

    #[test]
    fn surviving_image_post_can_have_a_deleted_author() {
        let mut value = post();
        value["authorInfo"] = Value::Null;
        value["url"] = json!("https://i.redd.it/surviving.jpg");
        value["media"] = json!({"still": {"source": {"url": "https://i.redd.it/surviving.jpg"}}});
        let response = listing(&[value], None).unwrap();
        let parsed = RedditPostParser::default().parse(&response);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].author, "[deleted]");
    }

    #[test]
    fn profile_deleted_and_text_posts_are_valid_without_media() {
        let mut profile = post();
        profile["__typename"] = json!("ProfilePost");
        profile["subreddit"] = Value::Null;
        profile["profile"] = json!({"name": "u_OriginalAuthor"});
        let title = profile
            .as_object_mut()
            .unwrap()
            .remove("postTitle")
            .unwrap();
        profile["title"] = title;
        let mut deleted = post();
        deleted["__typename"] = json!("DeletedSubredditPost");
        deleted.as_object_mut().unwrap().remove("authorInfo");
        deleted.as_object_mut().unwrap().remove("url");
        let mut text = post();
        text["isSelfPost"] = json!(true);
        text.as_object_mut().unwrap().remove("url");
        let response = listing(&[profile, deleted, text], None).unwrap();
        assert_eq!(response.data.children.len(), 3);
        assert_eq!(response.data.children[0].data.subreddit, "u_OriginalAuthor");
        assert_eq!(response.data.children[0].data.title, "Original title");
        assert_eq!(response.data.children[1].data.author, "[deleted]");
        assert!(RedditPostParser::default().parse(&response).is_empty());
    }
}

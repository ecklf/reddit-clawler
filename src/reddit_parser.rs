use crate::clients::api_types::reddit::submitted_response::{
    RedditSubmittedChild, RedditSubmittedChildData, RedditSubmittedResponse,
};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RedditMediaProviderType {
    RedditImage,
    RedditGifVideo,
    RedditVideo,
    RedditGalleryImage,
    ImgurImage,
    YoutubeVideo,
    RedgifsImage,
    RedgifsVideo,
    None,
}

#[derive(Debug, Clone)]
pub struct RedditCrawlerPost {
    pub author: String,
    pub created_utc: DateTime<Utc>,
    pub extension: String,
    pub id: String,
    pub provider: RedditMediaProviderType,
    pub subreddit: String,
    pub title: String,
    pub upvotes: i64,
    pub url: String,
    // This is the index of the image in the gallery
    pub index: Option<usize>,
}

#[derive(Default, Debug, Clone)]
pub struct RedditPostParser {}

impl RedditPostParser {
    pub fn parse(&self, response: &RedditSubmittedResponse) -> Vec<RedditCrawlerPost> {
        response
            .data
            .children
            .iter()
            .flat_map(|child| self.parse_user_submitted(child))
            .collect::<Vec<_>>()
    }

    fn parse_user_submitted(&self, child: &RedditSubmittedChild) -> Vec<RedditCrawlerPost> {
        let data = &child.data;
        let RedditSubmittedChildData {
            author,
            created_utc,
            is_gallery,
            is_reddit_media_domain,
            media,
            media_metadata,
            subreddit,
            title,
            ups: upvotes,
            is_video,
            ..
        } = data;

        // Set to `true` if the post is hosted on Reddit's own media domai.
        // This excludes gallery posts, which is also hosted there?
        match is_reddit_media_domain {
            // Handle Reddit posts with single images or videos
            true => {
                match is_video {
                    Some(true) => {
                        if let Some(m) = media {
                            if let Some(u) = &m.reddit_video {
                                return vec![
                                    (RedditCrawlerPost {
                                        author: author.to_owned(),
                                        created_utc: created_utc.to_owned(),
                                        extension: "mp4".to_owned(),
                                        id: data.id.to_owned(),
                                        index: None,
                                        provider: RedditMediaProviderType::RedditVideo,
                                        subreddit: subreddit.to_owned(),
                                        title: title.to_owned(),
                                        upvotes: upvotes.to_owned(),
                                        url: u.hls_url.to_owned(),
                                    }),
                                ];
                            }
                        }
                    }
                    Some(false) => {
                        let videos =
                            data.preview.as_ref().map(|preview| {
                                preview
                                    .images
                                    .iter()
                                    .filter_map(|image| {
                                        image.variants.mp4.as_ref().map(|mp4_src| {
                                            RedditCrawlerPost {
                                                author: author.to_owned(),
                                                created_utc: created_utc.to_owned(),
                                                extension: "mp4".to_owned(),
                                                id: data.id.to_owned(),
                                                index: None,
                                                provider: RedditMediaProviderType::RedditImage,
                                                subreddit: subreddit.to_owned(),
                                                title: title.to_owned(),
                                                upvotes: upvotes.to_owned(),
                                                url: mp4_src.source.url.to_owned(),
                                            }
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            });

                        if let Some(videos) = videos {
                            if !videos.is_empty() {
                                return videos;
                            }
                        }

                        let gifs =
                            data.preview.as_ref().map(|preview| {
                                preview
                                    .images
                                    .iter()
                                    .filter_map(|image| {
                                        image.variants.gif.as_ref().map(|gif_src| {
                                            RedditCrawlerPost {
                                                author: author.to_owned(),
                                                created_utc: created_utc.to_owned(),
                                                extension: "gif".to_owned(),
                                                id: data.id.to_owned(),
                                                index: None,
                                                provider: RedditMediaProviderType::RedditGifVideo,
                                                subreddit: subreddit.to_owned(),
                                                title: title.to_owned(),
                                                upvotes: upvotes.to_owned(),
                                                url: gif_src.source.url.to_owned(),
                                            }
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            });

                        if let Some(gifs) = gifs {
                            if !gifs.is_empty() {
                                return gifs;
                            }
                        }

                        let extension: String = data.url.split('.').rev().take(1).collect();

                        if extension == "gif" {
                            return vec![
                                (RedditCrawlerPost {
                                    author: author.to_owned(),
                                    created_utc: created_utc.to_owned(),
                                    extension: "gif".to_owned(),
                                    id: data.id.to_owned(),
                                    index: None,
                                    provider: RedditMediaProviderType::RedditImage,
                                    subreddit: subreddit.to_owned(),
                                    title: title.to_owned(),
                                    upvotes: upvotes.to_owned(),
                                    url: data.url.to_owned(),
                                }),
                            ];
                        }

                        return vec![
                            (RedditCrawlerPost {
                                author: author.to_owned(),
                                created_utc: created_utc.to_owned(),
                                extension: "webp".to_owned(),
                                id: data.id.to_owned(),
                                index: None,
                                provider: RedditMediaProviderType::RedditImage,
                                subreddit: subreddit.to_owned(),
                                title: title.to_owned(),
                                upvotes: upvotes.to_owned(),
                                url: data.url.to_owned(),
                            }),
                        ];
                    }
                    _ => {
                        // No-op, there may be more cases to handle
                    }
                }
            }
            // Handle all other media
            false => {
                // Handle Reddit posts with galleries
                if let (Some(media_metadata), Some(true)) = (media_metadata, is_gallery) {
                    if let Some(gallery_data) = &data.gallery_data {
                        let media_ids = gallery_data
                            .items
                            .iter()
                            .map(|item| item.media_id.to_owned())
                            .collect::<Vec<String>>();

                        return media_ids
                            .iter()
                            .enumerate()
                            .filter_map(|(i, media_id)| {
                                media_metadata.get(media_id).and_then(|media| {
                                    media.s.as_ref().and_then(|s_media| {
                                        if let Some(u) = &s_media.u {
                                            return Some(RedditCrawlerPost {
                                                author: author.to_owned(),
                                                created_utc: created_utc.to_owned(),
                                                extension: "webp".to_owned(),
                                                id: data.id.to_owned(),
                                                index: Some(i),
                                                provider:
                                                    RedditMediaProviderType::RedditGalleryImage,
                                                subreddit: subreddit.to_owned(),
                                                title: format!("{}-{}", title, i),
                                                upvotes: upvotes.to_owned(),
                                                url: u.to_owned(),
                                            });
                                        }

                                        None
                                    })
                                })
                            })
                            .collect::<Vec<_>>();
                    }
                }
                // Handle Reddit posts with mp4
                if let Some(media_metadata) = media_metadata {
                    let media_ids = media_metadata.keys().collect::<Vec<&String>>();

                    return media_ids
                        .iter()
                        .enumerate()
                        .filter_map(|(i, media_id)| {
                            media_metadata.get(*media_id).and_then(|media| {
                                media.s.as_ref().and_then(|s_media| {
                                    if let Some(mp4) = &s_media.mp4 {
                                        return Some(RedditCrawlerPost {
                                            author: author.to_owned(),
                                            created_utc: created_utc.to_owned(),
                                            extension: "mp4".to_owned(),
                                            id: data.id.to_owned(),
                                            index: Some(i),
                                            provider: RedditMediaProviderType::RedditGifVideo,
                                            subreddit: subreddit.to_owned(),
                                            title: format!("{}-{}", title, i),
                                            upvotes: upvotes.to_owned(),
                                            url: mp4.to_owned(),
                                        });
                                    }
                                    None
                                })
                            })
                        })
                        .collect::<Vec<_>>();
                }
                // Handle YouTube embeds
                if let Some(m) = media {
                    match &m.type_field {
                        Some(tf) if tf.eq("youtube.com") => {
                            return vec![
                                (RedditCrawlerPost {
                                    author: author.to_owned(),
                                    created_utc: created_utc.to_owned(),
                                    extension: "mp4".to_owned(),
                                    id: data.id.to_owned(),
                                    index: None,
                                    provider: RedditMediaProviderType::YoutubeVideo,
                                    subreddit: subreddit.to_owned(),
                                    title: title.to_owned(),
                                    upvotes: upvotes.to_owned(),
                                    url: data.url.to_owned(),
                                }),
                            ];
                        }
                        _ => {}
                    }
                }
                // Handle Redgifs images
                if data.url.contains("redgifs.com/i/") {
                    return vec![
                        (RedditCrawlerPost {
                            author: author.to_owned(),
                            created_utc: created_utc.to_owned(),
                            extension: "webp".to_owned(),
                            id: data.id.to_owned(),
                            index: None,
                            provider: RedditMediaProviderType::RedgifsImage,
                            subreddit: subreddit.to_owned(),
                            title: title.to_owned(),
                            upvotes: upvotes.to_owned(),
                            url: data.url.to_owned(),
                        }),
                    ];
                }
                // Handle Redgifs video embeds
                if data.url.contains("redgifs.com/watch/") || data.url.contains("redgifs.com/ifr/")
                {
                    return vec![
                        (RedditCrawlerPost {
                            author: author.to_owned(),
                            created_utc: created_utc.to_owned(),
                            extension: "mp4".to_owned(),
                            id: data.id.to_owned(),
                            index: None,
                            provider: RedditMediaProviderType::RedgifsVideo,
                            subreddit: subreddit.to_owned(),
                            title: title.to_owned(),
                            upvotes: upvotes.to_owned(),
                            url: data.url.to_owned(),
                        }),
                    ];
                }
                // Handle Imgur embeds
                if data.url.contains("imgur") {
                    let extension: String = data.url.split('.').rev().take(1).collect();
                    return vec![
                        (RedditCrawlerPost {
                            author: author.to_owned(),
                            created_utc: created_utc.to_owned(),
                            extension,
                            id: data.id.to_owned(),
                            index: None,
                            provider: RedditMediaProviderType::ImgurImage,
                            subreddit: subreddit.to_owned(),
                            title: title.to_owned(),
                            upvotes: upvotes.to_owned(),
                            url: data.url.to_owned(),
                        }),
                    ];
                }
            }
        }
        // All cases fell through, return empty vector
        Vec::with_capacity(0)
    }
}

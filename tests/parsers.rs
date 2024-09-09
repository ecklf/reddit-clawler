use pretty_assertions::assert_eq;
use reddit_clawler::{
    self,
    clients::api_types::reddit::submitted_response::RedditSubmittedResponse,
    reddit_parser::{RedditCrawlerPost, RedditMediaProviderType, RedditPostParser},
};
use std::{error::Error, fs};

#[test]
fn it_detects_reddit_image() -> Result<(), Box<dyn Error>> {
    let data = fs::read_to_string("./tests/mocks/reddit/submitted_response/reddit_image.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 1);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::RedditImage);
    }

    Ok(())
}

#[test]
fn it_detects_reddit_gallery() -> Result<(), Box<dyn Error>> {
    let data = fs::read_to_string("./tests/mocks/reddit/submitted_response/reddit_gallery.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 3);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::RedditGalleryImage);
    }

    Ok(())
}

#[test]
fn it_detects_reddit_video() -> Result<(), Box<dyn Error>> {
    let data = fs::read_to_string("./tests/mocks/reddit/submitted_response/reddit_video.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 1);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::RedditVideo);
    }

    Ok(())
}

#[test]
fn it_detects_imgur_image() -> Result<(), Box<dyn Error>> {
    let data = fs::read_to_string("./tests/mocks/reddit/submitted_response/imgur_image.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 1);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::ImgurImage);
    }

    Ok(())
}

#[test]
fn it_detects_youtube_video() -> Result<(), Box<dyn Error>> {
    let data = fs::read_to_string("./tests/mocks/reddit/submitted_response/youtube_video.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 1);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::YoutubeVideo);
    }

    Ok(())
}

#[test]
fn it_detects_redgifs_image() -> Result<(), Box<dyn Error>> {
    let data = fs::read_to_string("./tests/mocks/reddit/submitted_response/redgifs_image.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 1);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::RedgifsImage);
    }

    Ok(())
}

#[test]
fn it_detects_redgifs_video() -> Result<(), Box<dyn Error>> {
    let data = fs::read_to_string("./tests/mocks/reddit/submitted_response/redgifs_video.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 1);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::RedgifsVideo);
    }

    Ok(())
}

#[test]
fn it_detects_redgifs_video_iframe() -> Result<(), Box<dyn Error>> {
    let data =
        fs::read_to_string("./tests/mocks/reddit/submitted_response/redgifs_video_iframe.json")?;
    let responses: Vec<RedditSubmittedResponse> = serde_json::from_str(&data)?;
    let res = responses
        .first()
        .ok_or("Expected mockfile to contain a RedditUserSubmittedResponse")?;

    let post_parser = RedditPostParser::default();
    let parsed_posts = post_parser.parse(res);

    assert_eq!(parsed_posts.len(), 1);

    for mt in parsed_posts.iter() {
        let RedditCrawlerPost { provider, .. } = mt;
        assert_eq!(provider, &RedditMediaProviderType::RedgifsVideo);
    }

    Ok(())
}

//! Read-only persisted listing operations. `execute` returns the GraphQL data
//! object (not the response envelope). SDUI connections live under `elements`;
//! search connections contain full post nodes. Profile/search cursors are opaque
//! server values; subreddit SDUI feeds can use a base64-encoded last fullname.

use std::collections::HashSet;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

use super::{posts, GraphQlClient};
use crate::{
    cli::{CliRedditCommand, RedditCategoryFilter, RedditTimeframeFilter},
    clients::{
        api_types::reddit::submitted_response::RedditSubmittedResponse, reddit::RedditProviderError,
    },
};

type Result<T> = std::result::Result<T, RedditProviderError>;

const USER_PROFILE: (&str, &str) = (
    "UserProfile",
    "a05c628c5469c6b107c85988a8b1d0b79a2e7b6150ebd3a449c67e226e975ddc",
);
const SUBMITTED: (&str, &str) = (
    "SubmittedPostsFeedSdui",
    "d973adb1955e30c12e200bd2dbe843172276aebbae0ff1a8dda76f0ad19a0605",
);
const SUBREDDIT: (&str, &str) = (
    "SubredditFeedSdui",
    "45faffc9969af3d623a079e279c9d9a535b6f877809426a9a8ed203f046a23f0",
);
const POPULAR: (&str, &str) = (
    "PopularFeedSdui",
    "effee8db2d66287bea4e08a3ccf3a5fcb2db245c363be6fe50a3ba01e9962a24",
);
const ALL: (&str, &str) = (
    "AllFeedSdui",
    "dea8ebf08bd9127a580fd664b9c312e590a195690f5c04c32d60b9370d876fb0",
);
const POSTS_BY_IDS: (&str, &str) = (
    "PostsByIds",
    "b210ea1d852cdc2c25d9bff2d9fa8a406e1a968f36392798812f336694ac27f4",
);
const SEARCH: (&str, &str) = (
    "SearchPosts",
    "c3d31b88bfa15180fdcd0c8f6f13b8c0e131d0c981c9b9c8a7c537ddd0ed3348",
);

impl GraphQlClient<'_> {
    pub(in crate::clients::reddit) async fn user_submissions(
        &self,
        cmd: &CliRedditCommand,
    ) -> Result<Vec<RedditSubmittedResponse>> {
        if cmd.options.limit == Some(0) {
            return Ok(Vec::new());
        }
        let profile = self
            .execute(
                USER_PROFILE.0,
                USER_PROFILE.1,
                profile_variables(&cmd.resource),
            )
            .await?;
        check_profile(&profile)?;

        let mut variables = feed_variables(&cmd.category, &cmd.timeframe);
        variables["username"] = json!(cmd.resource);
        let mut responses = self
            .sdui_submissions(SUBMITTED, variables, true, cmd.options.limit)
            .await?;
        match cmd.category {
            RedditCategoryFilter::New | RedditCategoryFilter::Hot | RedditCategoryFilter::Top => {
                let supplemental = self
                    .search_submissions(
                        &format!("author:{}", cmd.resource),
                        &cmd.category,
                        &cmd.timeframe,
                        cmd.options.limit,
                    )
                    .await?;
                responses.extend(supplemental);
            }
            // SearchPosts has no RISING or CONTROVERSIAL sort. Keep the actual
            // submitted feed for these sorts, without silently searching NEW.
            RedditCategoryFilter::Rising | RedditCategoryFilter::Controversial => {}
        }
        deduplicate(&mut responses);
        if cmd.options.verbose {
            let post_count: usize = responses.iter().map(|page| page.data.children.len()).sum();
            eprintln!("GraphQL fetched {post_count} unique posts across {} listing pages (including author search)", responses.len());
        }
        Ok(responses)
    }

    pub(in crate::clients::reddit) async fn subreddit_submissions(
        &self,
        cmd: &CliRedditCommand,
    ) -> Result<Vec<RedditSubmittedResponse>> {
        let (operation, variables) = subreddit_query(&cmd.resource, &cmd.category, &cmd.timeframe);
        self.sdui_submissions(operation, variables, false, cmd.options.limit)
            .await
    }

    pub(in crate::clients::reddit) async fn search_submissions(
        &self,
        term: &str,
        category: &RedditCategoryFilter,
        timeframe: &RedditTimeframeFilter,
        limit: Option<u32>,
    ) -> Result<Vec<RedditSubmittedResponse>> {
        // Validate even with a zero limit: unsupported sorts must not silently
        // turn into a relevance/new search or an apparently successful result.
        let variables = search_variables(term, category, timeframe)?;
        let mut pages = Vec::new();
        let mut pagination = Pagination::default();
        while within_limit(pages.len(), limit) {
            let data = self
                .execute(
                    SEARCH.0,
                    SEARCH.1,
                    with_cursor(&variables, "afterCursor", pagination.after.as_deref()),
                )
                .await?;
            let connection = data
                .pointer("/search/general/posts")
                .ok_or(RedditProviderError::GraphQl("missing search posts root"))?;
            let (edges, next) = parse_connection(connection)?;
            let nodes = search_nodes(edges)?;
            pagination.advance(next)?;
            pages.push(posts::listing(&nodes, pagination.after.clone())?);
            if pagination.after.is_none() {
                break;
            }
        }
        deduplicate(&mut pages);
        Ok(pages)
    }

    async fn sdui_submissions(
        &self,
        operation: (&'static str, &'static str),
        variables: Value,
        profile: bool,
        limit: Option<u32>,
    ) -> Result<Vec<RedditSubmittedResponse>> {
        let mut pages = Vec::new();
        let mut pagination = Pagination::default();
        while within_limit(pages.len(), limit) {
            let data = self
                .execute(
                    operation.0,
                    operation.1,
                    with_cursor(&variables, "after", pagination.after.as_deref()),
                )
                .await?;
            let connection = sdui_connection(&data, profile)?;
            let (ids, next) = sdui_page(connection, profile)?;
            pagination.advance(next)?;
            let nodes = if ids.is_empty() {
                Vec::new()
            } else {
                let mut hydration = post_variables();
                hydration["ids"] = json!(ids);
                let data = self
                    .execute(POSTS_BY_IDS.0, POSTS_BY_IDS.1, hydration)
                    .await?;
                data.get("postsInfoByIds")
                    .and_then(Value::as_array)
                    .ok_or(RedditProviderError::GraphQl("missing postsInfoByIds array"))?
                    .clone()
            };
            // Count discovery pages, even empty/ad-only ones; hydration and
            // profile checks do not consume the limit. Never stop on no posts.
            pages.push(posts::listing(&nodes, pagination.after.clone())?);
            if pagination.after.is_none() {
                break;
            }
        }
        deduplicate(&mut pages);
        Ok(pages)
    }
}

fn profile_variables(name: &str) -> Value {
    json!({"name": name, "includePremiumAvatarTreatment": false})
}

fn check_profile(data: &Value) -> Result<()> {
    let profile = data
        .get("redditorInfoByName")
        .ok_or(RedditProviderError::GraphQl(
            "missing redditorInfoByName root",
        ))?;
    if profile.is_null() {
        return Err(RedditProviderError::NotFound);
    }
    match profile.get("__typename").and_then(Value::as_str) {
        Some("Redditor") => Ok(()),
        Some("DeletedRedditor") => Err(RedditProviderError::NotFound),
        Some("SuspendedRedditor" | "BannedRedditor") => Err(RedditProviderError::Suspended),
        // Unknown access/schema states must not permanently cache suspension.
        Some(_) => Err(RedditProviderError::GraphQl("unrecognized profile state")),
        None => Err(RedditProviderError::GraphQl("profile has no __typename")),
    }
}

fn feed_variables(category: &RedditCategoryFilter, timeframe: &RedditTimeframeFilter) -> Value {
    json!({
        "sort": category.to_string().to_uppercase(),
        "time": timeframe.to_string().to_uppercase(),
    })
}

fn subreddit_query(
    resource: &str,
    category: &RedditCategoryFilter,
    timeframe: &RedditTimeframeFilter,
) -> ((&'static str, &'static str), Value) {
    let mut variables = feed_variables(category, timeframe);
    let operation = match resource.to_ascii_lowercase().as_str() {
        "popular" => POPULAR,
        "all" => ALL,
        _ => {
            variables["subredditName"] = json!(resource);
            SUBREDDIT
        }
    };
    (operation, variables)
}

fn with_cursor(variables: &Value, key: &str, after: Option<&str>) -> Value {
    let mut variables = variables.clone();
    if let Some(after) = after {
        variables[key] = json!(after);
    }
    variables
}

fn search_variables(
    term: &str,
    category: &RedditCategoryFilter,
    timeframe: &RedditTimeframeFilter,
) -> Result<Value> {
    let sort = match category {
        RedditCategoryFilter::Hot => "HOT",
        RedditCategoryFilter::New => "NEW",
        RedditCategoryFilter::Top => "TOP",
        RedditCategoryFilter::Rising | RedditCategoryFilter::Controversial => {
            return Err(RedditProviderError::GraphQl(
                "SearchPosts does not support Rising or Controversial sorts",
            ));
        }
    };
    let mut variables = post_variables();
    // Only the flags declared by SearchPosts' post fragment are accepted.
    for key in [
        "includePcpV2InPosts",
        "includeDeletedPosts",
        "includeUnavailablePostReason",
        "commentId",
        "includeCommentId",
        "includeInterestTags",
        "includePostTombstone",
        "includeAutomatedFeedback",
    ] {
        variables.as_object_mut().unwrap().remove(key);
    }
    variables["query"] = json!(term);
    variables["productSurface"] = json!("android");
    variables["pageSize"] = json!(25);
    variables["sort"] = json!(sort);
    variables["filters"] = json!([{"key": "time_range", "value": timeframe.to_string()}]);
    Ok(variables)
}

fn post_variables() -> Value {
    json!({
        "includeAwards": true,
        "includeSubredditInPosts": true,
        "includePcpV2InPosts": true,
        "includePostStats": true,
        "includeDeletedPosts": true,
        "includeCurrentUserAwards": false,
        "includeStillMediaAltText": true,
        "includeExtraStillResolutions": true,
        "includeExtendedVideoAsset": false,
        "includePlaybackMp4s": true,
        "includeMuxedMp4s": true,
        "includeDevvitData": true,
        "includeCommunityStatus": true,
        "includeIsModRemovedContentReadAllowed": true,
        "includeVideoPlaybackInComments": false,
        "includeUnavailablePostReason": true,
        "commentId": "",
        "includeCommentId": false,
        "includePredictedRemovalReasons": true,
        "includeSubredditBackgroundColor": true,
        "includeOverlayData": true,
        "includeCtaEnrichedPageTitle": true,
        "includePlaCards": true,
        "includePlaCardThumbnail": false,
        "includeWebviewPrefetchField": true,
        "includeAdBrowserDestinationField": true,
        "includeCollectionsHeroCard": true,
        "includeGalleryAdUrl": true,
        "includeCtaMediaColor": true,
        "includeFollowedForNotificationsStatus": true,
        "includeRedditHandleInfo": true,
        "includeVerificationStatus": true,
        "skipModerationInfo": false,
        "includeIsReceivingPostReplies": false,
        "includeIsActiveComments": true,
        "includeMediaSynthesis": true,
        "skipRedditPlayPlaybackMp4s": true,
        "includeLinkPreviewTitle": false,
        "includeInterestTags": false,
        "includePostTombstone": true,
        "includeAutomatedFeedback": false,
    })
}

fn within_limit(pages: usize, limit: Option<u32>) -> bool {
    limit.is_none_or(|limit| (pages as u64) < u64::from(limit))
}

#[derive(Default)]
struct Pagination {
    after: Option<String>,
    seen: HashSet<String>,
}

impl Pagination {
    fn advance(&mut self, next: Option<String>) -> Result<()> {
        if let Some(cursor) = &next {
            if !self.seen.insert(cursor.clone()) {
                return Err(RedditProviderError::GraphQl(
                    "listing cursor cycle detected",
                ));
            }
        }
        self.after = next;
        Ok(())
    }
}

fn sdui_connection(data: &Value, profile: bool) -> Result<&Value> {
    let root = if profile {
        data.get("profileFeed")
            .ok_or(RedditProviderError::GraphQl("missing profileFeed root"))?
    } else {
        // The upstream feed operations select one root, but do not share a
        // root field name. Require exactly one (ignoring optional metadata)
        // rather than guessing a name or accepting a missing feed as empty.
        let object = data
            .as_object()
            .ok_or(RedditProviderError::GraphQl("feed data is not an object"))?;
        let mut roots = object
            .iter()
            .filter(|(key, _)| key.as_str() != "__typename");
        let (_, root) = roots
            .next()
            .ok_or(RedditProviderError::GraphQl("missing SDUI feed root"))?;
        if roots.next().is_some() {
            return Err(RedditProviderError::GraphQl("ambiguous SDUI feed root"));
        }
        root
    };
    root.get("elements").ok_or(RedditProviderError::GraphQl(
        "missing SDUI elements connection",
    ))
}

fn sdui_page(connection: &Value, profile: bool) -> Result<(Vec<String>, Option<String>)> {
    let edges = connection
        .get("edges")
        .and_then(Value::as_array)
        .ok_or(RedditProviderError::GraphQl("listing has no edges array"))?;
    let ids = sdui_post_ids(edges)?;
    let next = if profile || connection.get("pageInfo").is_some() {
        // Profile feeds expose an opaque pageInfo cursor.
        parse_connection(connection)?.1
    } else {
        // Subreddit/Popular/All SDUI operations do not require pageInfo:
        // Reddit's app uses a base64-encoded last post fullname as `after`.
        ids.last().map(|id| STANDARD.encode(id))
    };
    Ok((ids, next))
}

fn parse_connection(connection: &Value) -> Result<(&[Value], Option<String>)> {
    let edges = connection
        .get("edges")
        .and_then(Value::as_array)
        .ok_or(RedditProviderError::GraphQl("listing has no edges array"))?;
    let info = connection
        .get("pageInfo")
        .ok_or(RedditProviderError::GraphQl("listing has no pageInfo"))?;
    let has_next =
        info.get("hasNextPage")
            .and_then(Value::as_bool)
            .ok_or(RedditProviderError::GraphQl(
                "pageInfo has no boolean hasNextPage",
            ))?;
    // On a terminal page Reddit can omit endCursor or return ""/null. It
    // isn't needed to retain this page's posts and must not fail the crawl.
    if !has_next {
        return match info.get("endCursor") {
            None | Some(Value::Null | Value::String(_)) => Ok((edges, None)),
            _ => Err(RedditProviderError::GraphQl(
                "pageInfo has invalid endCursor",
            )),
        };
    }
    let cursor = info
        .get("endCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .ok_or(RedditProviderError::GraphQl(
            "pageInfo hasNextPage=true but has no usable endCursor",
        ))?;
    Ok((edges, Some(cursor.to_owned())))
}

fn edge_node(edge: &Value) -> Result<&Value> {
    let node = edge
        .get("node")
        .ok_or(RedditProviderError::GraphQl("listing edge has no node"))?;
    if !node.is_null() && !node.is_object() {
        return Err(RedditProviderError::GraphQl(
            "listing node is not an object",
        ));
    }
    Ok(node)
}

fn sdui_post_ids(edges: &[Value]) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for edge in edges {
        let node = edge_node(edge)?;
        if node.get("adPayload").is_some_and(|ad| !ad.is_null()) {
            continue;
        }
        let id = node
            .get("groupId")
            .filter(|id| !id.is_null())
            .or_else(|| node.get("id"));
        if let Some(id) = id.and_then(Value::as_str) {
            if id.starts_with("t3_") && id.len() > 3 && seen.insert(id.to_owned()) {
                ids.push(id.to_owned());
            }
        }
    }
    Ok(ids)
}

fn search_nodes(edges: &[Value]) -> Result<Vec<Value>> {
    edges.iter().map(|edge| edge_node(edge).cloned()).collect()
}

fn deduplicate(pages: &mut [RedditSubmittedResponse]) {
    let mut seen = HashSet::new();
    for page in pages {
        page.data
            .children
            .retain(|child| seen.insert(child.data.id.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(edges: Value, has_next: bool, cursor: Value) -> Value {
        json!({"edges": edges, "pageInfo": {"hasNextPage": has_next, "endCursor": cursor}})
    }

    #[test]
    fn opaque_cursor_survives_empty_and_ad_only_pages() {
        let cursor = " opaque+/==:not-a-fullname ";
        for edges in [
            json!([]),
            json!([{"node": {"adPayload": {}, "id": "t3_ad"}}]),
        ] {
            let page = connection(edges, true, json!(cursor));
            let (edges, next) = parse_connection(&page).unwrap();
            assert!(sdui_post_ids(edges).unwrap().is_empty());
            let mut pagination = Pagination::default();
            pagination.advance(next).unwrap();
            assert_eq!(pagination.after.as_deref(), Some(cursor));
            for key in ["after", "afterCursor"] {
                assert_eq!(
                    with_cursor(&json!({}), key, pagination.after.as_deref())[key],
                    cursor
                );
            }
        }
    }

    #[test]
    fn pagination_rejects_malformed_and_detects_long_cycles() {
        for page in [
            json!(null),
            json!({"edges": []}),
            json!({"edges": [], "pageInfo": {"hasNextPage": true}}),
            connection(json!(null), false, Value::Null),
            connection(json!([]), true, Value::Null),
            connection(json!([]), true, json!("")),
            connection(json!([]), false, json!(5)),
            json!({"edges": [], "pageInfo": {"hasNextPage": "true", "endCursor": "a"}}),
        ] {
            assert!(parse_connection(&page).is_err(), "accepted {page}");
        }
        let mut pagination = Pagination::default();
        pagination.advance(Some("a".into())).unwrap();
        assert!(pagination.advance(Some("a".into())).is_err());
        let mut pagination = Pagination::default();
        pagination.advance(Some("a".into())).unwrap();
        pagination.advance(Some("b".into())).unwrap();
        assert!(pagination.advance(Some("a".into())).is_err());
        for cursor in [Value::Null, json!("terminal")] {
            assert!(parse_connection(&connection(json!([]), false, cursor))
                .unwrap()
                .1
                .is_none());
        }
    }

    #[test]
    fn terminal_pages_allow_missing_empty_and_null_cursor_without_losing_edges() {
        let edges = json!([{"node": {"groupId": "t3_kept"}}]);
        for cursor in [
            None,
            Some(Value::Null),
            Some(json!("")),
            Some(json!("unused")),
        ] {
            let mut page = json!({"edges": edges, "pageInfo": {"hasNextPage": false}});
            if let Some(cursor) = cursor {
                page["pageInfo"]["endCursor"] = cursor;
            }
            let (actual_edges, next) = parse_connection(&page).unwrap();
            assert_eq!(actual_edges, edges.as_array().unwrap());
            assert!(next.is_none());
            let (ids, next) = sdui_page(&page, true).unwrap();
            assert_eq!(ids, ["t3_kept"]);
            assert!(next.is_none());
        }
    }

    #[test]
    fn sdui_edges_exclude_ads_nonposts_and_duplicate_cells() {
        let edges = json!([
            {"node": {"groupId": "t3_one", "id": "cell"}},
            {"node": {"groupId": "t3_one"}},
            {"node": {"id": "t3_two", "adPayload": null}},
            {"node": {"groupId": null, "id": "t3_three"}},
            {"node": {"id": "t3_ad", "adPayload": {}}},
            {"node": {"id": "t3_ad2", "adPayload": false}},
            {"node": {"id": "t1_comment"}},
            {"node": {"groupId": "other", "id": "t3_not_a_group"}},
            {"node": null}, {"node": {"id": "t3_"}}
        ]);
        assert_eq!(
            sdui_post_ids(edges.as_array().unwrap()).unwrap(),
            ["t3_one", "t3_two", "t3_three"]
        );
        assert!(sdui_post_ids(&[json!({})]).is_err());
        assert!(search_nodes(&[json!({"node": 7})]).is_err());
        assert_eq!(
            search_nodes(&[json!({"node": {"id": "t3_x"}})]).unwrap(),
            [json!({"id": "t3_x"})]
        );
    }

    #[test]
    fn missing_and_ambiguous_roots_are_errors() {
        for data in [json!({}), json!({"profileFeed": null})] {
            assert!(sdui_connection(&data, true).is_err());
        }
        for data in [json!({}), json!({"a": {}, "b": {}}), json!({"feed": null})] {
            assert!(sdui_connection(&data, false).is_err());
        }
        let data = json!({"anyFeedName": {"elements": connection(json!([]), false, Value::Null)}});
        assert!(parse_connection(sdui_connection(&data, false).unwrap()).is_ok());
    }

    #[test]
    fn profile_status_and_variables() {
        assert_eq!(
            profile_variables("MixedCase"),
            json!({"name": "MixedCase", "includePremiumAvatarTreatment": false})
        );
        assert!(check_profile(&json!({"redditorInfoByName": {"__typename": "Redditor"}})).is_ok());
        for profile in [Value::Null, json!({"__typename": "DeletedRedditor"})] {
            assert!(matches!(
                check_profile(&json!({"redditorInfoByName": profile})),
                Err(RedditProviderError::NotFound)
            ));
        }
        assert!(matches!(
            check_profile(&json!({"redditorInfoByName": {"__typename": "SuspendedRedditor"}})),
            Err(RedditProviderError::Suspended)
        ));
        assert!(check_profile(&json!({})).is_err());
        assert!(check_profile(&json!({"redditorInfoByName": {}})).is_err());
        assert!(matches!(
            check_profile(&json!({"redditorInfoByName": {"__typename": "FutureAccessGate"}})),
            Err(RedditProviderError::GraphQl(_))
        ));
    }

    #[test]
    fn subreddit_pages_without_page_info_use_encoded_last_fullname() {
        let page = json!({"edges": [
            {"node": {"groupId": "t3_one"}},
            {"node": {"groupId": "t3_two"}},
            {"node": {"id": "t3_ad", "adPayload": {}}}
        ]});
        let (ids, next) = sdui_page(&page, false).unwrap();
        assert_eq!(ids, ["t3_one", "t3_two"]);
        assert_eq!(next, Some(STANDARD.encode("t3_two")));
        assert!(sdui_page(&page, true).is_err());
        assert_eq!(
            sdui_page(&json!({"edges": []}), false).unwrap(),
            (vec![], None)
        );
    }

    #[test]
    fn subreddit_operations_have_exact_variables() {
        use RedditCategoryFilter::Hot;
        use RedditTimeframeFilter::All;
        for (name, expected) in [("PoPuLaR", POPULAR), ("ALL", ALL)] {
            let (operation, vars) = subreddit_query(name, &Hot, &All);
            assert_eq!(operation, expected);
            assert_eq!(vars, json!({"sort": "HOT", "time": "ALL"}));
            assert_eq!(with_cursor(&vars, "after", None), vars);
        }
        let (operation, vars) = subreddit_query("Rust", &Hot, &All);
        assert_eq!(operation, SUBREDDIT);
        assert_eq!(
            vars,
            json!({"sort": "HOT", "time": "ALL", "subredditName": "Rust"})
        );
    }

    #[test]
    fn deduplication_spans_pages_and_supplemental_results_without_changing_cursors() {
        use crate::clients::api_types::reddit::submitted_response::RedditSubmittedChild;
        let mut pages = Vec::new();
        for ids in [["one", "two"], ["two", "three"], ["one", "three"]] {
            let mut page = RedditSubmittedResponse::default();
            page.data.after = Some("opaque".into());
            for id in ids {
                let mut child = RedditSubmittedChild::default();
                child.data.id = id.into();
                page.data.children.push(child);
            }
            pages.push(page);
        }
        deduplicate(&mut pages);
        assert_eq!(pages[0].data.children.len(), 2);
        assert_eq!(pages[1].data.children.len(), 1);
        assert_eq!(pages[1].data.children[0].data.id, "three");
        assert!(pages[2].data.children.is_empty());
        assert!(pages
            .iter()
            .all(|page| page.data.after.as_deref() == Some("opaque")));
    }

    #[test]
    fn feed_and_search_variables_use_distinct_enums_and_flags() {
        use RedditCategoryFilter::{Controversial, Hot, New, Rising, Top};
        use RedditTimeframeFilter::{All, Week};
        assert_eq!(
            feed_variables(&Top, &Week),
            json!({"sort": "TOP", "time": "WEEK"})
        );
        for (category, sort) in [(Hot, "HOT"), (New, "NEW"), (Top, "TOP")] {
            let vars = search_variables("author:Name", &category, &Week).unwrap();
            assert_eq!(vars["query"], "author:Name");
            assert_eq!(vars["sort"], sort);
            assert_eq!(vars["pageSize"], 25);
            assert_eq!(vars["productSurface"], "android");
            assert_eq!(
                vars["filters"],
                json!([{"key": "time_range", "value": "week"}])
            );
            assert!(vars.get("afterCursor").is_none());
            assert!(vars.get("time").is_none());
            assert_eq!(vars.as_object().unwrap().len(), 38); // 33 fragment flags + 5 search parameters.
            assert_eq!(vars["includeAwards"], true);
            assert_eq!(vars["skipModerationInfo"], false);
            assert_eq!(vars["skipRedditPlayPlaybackMp4s"], true);
            for key in [
                "includePcpV2InPosts",
                "includeDeletedPosts",
                "includeUnavailablePostReason",
                "commentId",
                "includeCommentId",
                "includeInterestTags",
                "includePostTombstone",
                "includeAutomatedFeedback",
            ] {
                assert!(post_variables().get(key).is_some());
                assert!(vars.get(key).is_none());
            }
        }
        assert!(search_variables("x", &Rising, &All).is_err());
        assert!(search_variables("x", &Controversial, &All).is_err());
        assert!(within_limit(0, None));
        assert!(!within_limit(0, Some(0)));
        assert!(within_limit(1, Some(2)));
        assert!(!within_limit(2, Some(2)));
    }
}

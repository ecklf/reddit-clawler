# Development

[Back to the README](README.md).

## Rate limiting

Querying posts is paginated and can lead to rate limiting. `--limit` bounds listing
pages per feed, not total HTTP requests. GraphQL uses Reddit's server-selected
feed page sizes (search requests 25 posts); each feed page may require another
request to fetch full post details. User crawls also perform a supplemental
author search with its own page limit. Legacy JSON pages request up to 100 posts.

## HTTP transport

Requests use a Safari iOS browser profile through `wreq`, keeping the TLS handshake,
HTTP/2 settings, header ordering, and user agent consistent. The shared client reuses
connections and session cookies and supports gzip, Brotli, and deflate responses.

GET and HEAD requests retry transient connection failures, timeouts, and HTTP
408/429/500/502/503/504 responses up to three times with exponential backoff and
jitter. `Retry-After` is honored in both seconds and HTTP-date form; delays over
two minutes return the response instead of retrying early.

With `--cookies`, the crawler uses Reddit's persisted GraphQL queries, following
[Hydra's protocol](https://github.com/dmilin1/hydra/blob/master/api/gql/GqlApi.ts).
It reuses `token_v2` when it is not known to be expired, or renews it using the
exported `reddit_session`. Numeric expiry claims support fractional seconds; a
token with an unreadable expiry is tried against Reddit before renewal rather
than rejected locally. Known account mismatches still trigger renewal:
a HEAD request primes the web cookies, then an empty POST to
`/svc/shreddit/graphql` obtains `token_v2` (Reddit may return 400 while setting it).
Post IDs are discovered through SDUI feeds and hydrated through `PostsByIds`;
search uses `SearchPosts`. Images, ordered galleries, video, and external media
are converted to the existing parser/download pipeline with legacy cache IDs.

Only read operations are sent. A rejected imported token is refreshed and retried
at most once; a newly issued rejected token fails rather than looping. GraphQL
POSTs do not use the transport's automatic transient retries. Tokens remain in
memory and are not printed or written back to the cookie file. GraphQL errors
fail the crawl instead of caching incomplete results as deleted resources.
Persisted query hashes can change, and valid cookies do not guarantee Reddit
will allow token renewal or API access. GraphQL search supports `new`, `hot`,
and `top`; user crawls omit supplemental search for `rising`/`controversial`.
Subreddit access gates are not automatically opted into.

If token renewal fails, the error reports only safe metadata: the imported token's
validity category, HEAD/proxy HTTP statuses, the proxy response type, whether
`token_v2` appeared in a Set-Cookie header, and whether a usable token reached the
jar. Cookie values, JWT claims, raw headers, URLs, and response bodies are never
included. This distinguishes a server not issuing a token from a token-format,
expiry/clock, or cookie-scope problem. A `reddit_session` cookie being present does
not by itself prove the session is still accepted by Reddit. You can also export
a fresh `token_v2` from your logged-in browser alongside `reddit_session` to avoid
renewal until that token expires. Do not share the cookie export itself.

Without `--cookies`, legacy Reddit JSON requests (user listings, user details, subreddit listings, and search)
explicitly accept JSON and fall back once to the same endpoint on `old.reddit.com`
if `www.reddit.com` returns 403, redirects to a JavaScript challenge
(`js_challenge=1`), or serves HTML with a successful status. The fallback preserves
the original query and pagination and reuses the cookie jar without widening
host-only cookies. Login redirects, not-found responses, and rate limits do not
trigger this fallback.

If both hosts reject the request, the crawler reports authentication, challenge,
or forbidden errors rather than treating an HTML page as JSON. Login redirects
(including a login page returning 404) and HTML 404s are not treated as deleted
users. If an earlier crawl incorrectly cached a user as deleted, retry with
`--force` to fetch it again.

Complete any challenge in your browser on the same network and export fresh
Reddit cookies; the crawler does not execute browser challenges. Challenge URLs
can contain session-related tokens and should not be shared publicly.

Connections time out after 15 seconds, stalled reads after 30 seconds, and each
attempt (including the response body) after five minutes. Browser emulation can
reduce fingerprint mismatches, but sites may still require valid cookies or
interactive browser challenges.

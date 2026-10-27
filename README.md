# Reddit Clawler 🐾

A command-line tool written in Rust that crawls Reddit posts from a user, subreddit, or search term.

## Usage

Install the following dependencies:

- [yt-dlp](https://github.com/yt-dlp/yt-dlp)


## Commands

You can see all available commands by running:

```sh
./reddit_clawler --help
```

By default, the tool will download posts to the `output/{subcommand}/{value}` folder 

### User
Crawls posts from `/u/spez` with spawning `50` tasks to `./downloads/user/spez`:

```sh
./reddit_clawler user spez  --category new --tasks 50 -o ./downloads
```

If Reddit returns a forbidden response, provide an exported browser cookie file.
The file is first parsed as browser-export JSON, then as Netscape text if JSON parsing fails:

```sh
./reddit_clawler user spez --category new --cookies ./cookies.json
./reddit_clawler user spez --category new -c ./cookies.txt
```

Cookie files contain account credentials and should be kept private.

### Subreddit 
Crawls posts from `/r/redpandas` from the `top` category, filtered by `hour`:

```sh
./reddit_clawler subreddit redpandas --category top --timeframe hour
```

### Search 
Crawls posts for search term `olympics` from the `top` category, filtered by `hour`:

```sh
./reddit_clawler search olympics --category top --timeframe hour
```

## Features

### Providers (these are the most common I found):

- [x] Reddit Media
- [x] Imgur Media
- [x] YouTube Videos
- [x] Redgifs Videos

### Caching

After the downloads have finished, a `cache.json` file will be created in the folder of the downloaded resource.
This file keeps track of the posts you have already downloaded and skips downloading them on subsequent runs.

### Rate limiting

Querying posts is paginated (100 items per requests) and can lead to rate limiting.
To avoid this, you can provide a `--limit` flag to limit the number of requests for fetching a resource.
This can be useful for subsequent crawling.

### HTTP transport

Requests use a Safari iOS browser profile through `wreq`, keeping the TLS handshake,
HTTP/2 settings, header ordering, and user agent consistent. The shared client reuses
connections and session cookies and supports gzip, Brotli, and deflate responses.

GET and HEAD requests retry transient connection failures, timeouts, and HTTP
408/429/500/502/503/504 responses up to three times with exponential backoff and
jitter. `Retry-After` is honored in both seconds and HTTP-date form; delays over
two minutes return the response instead of retrying early. Authentication and
forbidden responses are returned without automatic retries.

Search requests (including the supplemental author search in user crawls) fall
back to `old.reddit.com` if `www.reddit.com` returns 403. Reddit can block search
on the main host even when the same cookies work for listings and old Reddit.
The fallback preserves the query and pagination; it does not bypass login or
access restrictions if old Reddit also rejects the request.

Connections time out after 15 seconds, stalled reads after 30 seconds, and each
attempt (including the response body) after five minutes. Browser emulation can
reduce fingerprint mismatches, but sites may still require valid cookies or
interactive browser challenges.

### File format

By default it will prefer `mp4` over `gif`, if available.

## Planned

- [ ] Providing custom filename scheme
- [ ] Configuration for conversion to other/small formats (`avif`/`webp`/`webm`)
- [ ] Remove duplicated

## Development

Building the HTTP transport requires a C/C++ compiler, CMake, Perl, and libclang
for its BoringSSL backend. On Debian/Ubuntu, install them with:

```sh
sudo apt-get install build-essential cmake perl pkg-config libclang-dev
```

You can use the `--skip` flag to skip the download process:

```sh
cargo run -- user spez --skip
```

You can use the `--mock` flag to provide a mock file for the responses of the Reddit client:

```sh
cargo run -- user spez --mock ./tests/mocks/reddit/submitted_response/reddit_video.json
```

## License

Reddit Clawler is licensed under the GNU General Public License v3.0. See the LICENSE file for details.

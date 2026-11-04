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
Crawls posts from `/u/spez` with up to `50` concurrent download tasks to
`./downloads/user/spez`:

```sh
./reddit_clawler user spez --category new --tasks 50 -o ./downloads
```

For authenticated GraphQL crawling, provide cookies exported from a logged-in
Reddit browser session. Include `reddit_session` and, when available, `token_v2`.
The crawler can reuse an accepted token or attempt to renew it using the session;
renewal is not guaranteed if Reddit blocks the request. Supplying `--cookies`
selects GraphQL for user, subreddit, and search commands instead of the legacy
`.json` listings, which Reddit may reject even with cookies. The file is first
parsed as browser-export JSON, then as Netscape text if JSON parsing fails:

```sh
./reddit_clawler user spez --category new --cookies ./cookies.json
./reddit_clawler user spez --category new -c ./cookies.txt
```

Cookie files contain account credentials and should be kept private. The crawler
uses them in memory and does not write renewed tokens back to the export.

To check fetching without downloading media, run:

```sh
cargo run --locked -- user spez --category new --limit 1 \
  --cookies ./cookies.txt --skip --verbose
```

`--verbose` reports the unique post count and listing-page count for GraphQL user
crawls, including supplemental author search. Text and ordinary link posts count
as fetched posts but may have no supported downloadable media, so a successful
fetch can still report `trying to download 0 posts`.

Add `--force` if an earlier run incorrectly cached the user as deleted or
suspended. For a normal download run, omit `--skip`; omit `--limit` to continue
through all available pages. The limited `spez` fetch has been verified live;
full-history fetching and media downloads were not part of that check.

### Subreddit 
Crawls posts from `/r/redpandas` from the `top` category, filtered by `hour`:

```sh
./reddit_clawler subreddit redpandas --category top --timeframe hour --cookies ./cookies.txt
```

### Search 
Crawls posts for search term `olympics` from the `top` category, filtered by `hour`:

```sh
./reddit_clawler search olympics --category top --timeframe hour --cookies ./cookies.txt
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

### File format

By default it will prefer `mp4` over `gif`, if available.

## Planned

- [ ] Providing custom filename scheme
- [ ] Configuration for conversion to other/small formats (`avif`/`webp`/`webm`)
- [ ] Remove duplicated

## Development

See [development.md](development.md) for HTTP transport, authentication, and
rate-limiting details.

Building the HTTP transport requires a C/C++ compiler, CMake, Perl, and libclang
for its BoringSSL backend. On Debian/Ubuntu, install them with:

```sh
sudo apt-get install build-essential cmake perl pkg-config libclang-dev
```

You can use the `--skip` flag to skip the download process:

```sh
cargo run --locked -- user spez --category new --cookies ./cookies.txt --skip
```

You can use the `--mock` flag to provide a mock file for the responses of the Reddit client:

```sh
cargo run --locked -- user spez --category new --mock ./tests/mocks/reddit/submitted_response/reddit_video.json
```

## License

Reddit Clawler is licensed under the GNU General Public License v3.0. See the LICENSE file for details.

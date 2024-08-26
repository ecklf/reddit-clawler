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

- [x] Reddit Images
- [x] Reddit Videos
- [x] Reddit Galleries
- [x] Imgur Images
- [x] Imgur Videos
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

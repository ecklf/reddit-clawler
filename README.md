# Reddit Clawler 🐾

A command-line tool written in Rust that crawls Reddit posts from a user or subreddit.

## Usage

You can see all available commands by running:

```sh
./reddit_clawler --help
```

Download posts from `/u/spez` with spawning `50` tasks emitting to `output/spez`:

```sh
./reddit_clawler user spez --tasks 50 -o output
```

Download posts from `/r/redpandas` from the `top` tab filtered by `hour`:

```sh
./reddit_clawler subreddit redpandas --category top --timeframe hour
```

## Features

Currently supports these providers (these are the most common I found):

- [x] Reddit (Images, Videos, Gallery)
- [x] Imgur
- [x] Redgifs

## Planned

- [ ] YouTube download
- [ ] Filename scheme
- [ ] Local caching
- [ ] Auto conversion to small formats (avif/webp)
- [ ] Remove duplicated

## Development

You can use the `--skip` flag to skip the download process:

```sh
./reddit_clawler user spez --skip
```

## License

Reddit Clawler is licensed under the GNU General Public License v3.0. See the LICENSE file for details.

# hentai-crawler

> [中文文档](README.zh.md)

A fast, async Rust CLI tool that uses a **4-stage pipeline** to scrape, download, convert, and pack an entire gallery from [ehentai.to](https://ehentai.to) or [e-hentai.org](https://e-hentai.org) — all four stages running simultaneously — producing a `.cbz` file ready for any comic reader.

## Features

- Supports both **ehentai.to** and **e-hentai.org**
- Accepts gallery URL or viewer page URL as input
- Automatically detects total page count and handles paginated gallery listings
- **Pipeline architecture** — scraping, downloading, PNG conversion, and CBZ packing all run concurrently and overlap with each other, so no stage has to wait for the previous one to fully finish
- Concurrent downloads and page scraping with configurable parallelism (defaults to all logical CPUs)
- Converts any image format (WebP, JPEG, GIF, etc.) to PNG using the blocking thread pool, saturating all CPU cores
- Four real-time progress bars, one per pipeline stage
- Interactive shell mode — run without arguments to download multiple galleries in one session
- Optional cookie authentication for e-hentai.org

## Pipeline

```
Stage 1 [scraping]  ████████░░░░░░░░░░░░░░░░░░
Stage 2 [download]    ░░░░████████████░░░░░░░░
Stage 3 [convert]         ░░░░░░░████████████░
Stage 4 [packing]               ░░░░░░░░░████
```

Each stage feeds the next via a bounded channel. As soon as an image URL is found it starts downloading; as soon as raw bytes arrive they are converted; no waiting for the whole gallery to finish each phase.

## Quick Start

### Build

```bash
git clone <repo>
cd ehentai-crawler
cargo build --release
```

### Usage

```bash
# ehentai.to
./target/release/ehentai-crawler "https://ehentai.to/g/<id>"
./target/release/ehentai-crawler "https://ehentai.to/g/<id>/<page>/"

# e-hentai.org
./target/release/ehentai-crawler "https://e-hentai.org/g/<id>/<token>/"
./target/release/ehentai-crawler "https://e-hentai.org/g/<id>/<token>/<page>/"

# Interactive shell mode (no arguments)
./target/release/ehentai-crawler
```

### Options

```
Usage: ehentai-crawler [OPTIONS] [URL]

Arguments:
  [URL]  Gallery URL (omit to enter interactive mode)

Options:
  -o, --output <OUTPUT>          Output directory [default: .]
  -c, --concurrency <N>          Concurrent downloads [default: logical CPU count]
      --cookies <COOKIES>        Cookie string for e-hentai.org authentication
                                 e.g. "ipb_member_id=123; ipb_pass_hash=abc; igneous=xyz"
  -h, --help                     Print help
  -V, --version                  Print version
```

### Example

```bash
./target/release/ehentai-crawler "https://e-hentai.org/g/<id>/<token>/" -o ~/Downloads
```

```
Fetching gallery info...
Gallery: <title>  |  Total pages: <N>
⠸ scraping  [=====================================>-] 56/60 (1s)
⠼ download  [=================================>-----] 57/60 (2s)
⠴ convert   [===========================>-----------] 58/60 (3s)
⠦ packing   [======================>----------------] 59/60 (3s)
Saved to: /home/user/Downloads/<id>.cbz
```

## Supported URL Formats

| Site | Format |
|------|--------|
| ehentai.to | `https://ehentai.to/g/<id>` |
| ehentai.to | `https://ehentai.to/g/<id>/<page>/` |
| e-hentai.org | `https://e-hentai.org/g/<id>/<token>/` |
| e-hentai.org | `https://e-hentai.org/g/<id>/<token>/<page>/` |

## Requirements

- Rust 1.75+
- OpenSSL (for TLS)

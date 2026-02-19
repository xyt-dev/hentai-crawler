# ehentai-crawler

> [中文文档](README.zh.md)

A fast, async Rust CLI tool that downloads an entire gallery from [ehentai.to](https://ehentai.to), converts all images to PNG, and packages them into a `.cbz` file ready for any comic reader.

## Features

- Accepts gallery ID, gallery URL, or viewer page URL as input
- Automatically detects total page count
- Concurrent page scraping and image downloading
- Converts any image format (WebP, JPEG, GIF, etc.) to PNG
- Packs output into a `.cbz` archive with zero-padded filenames for correct ordering
- Progress bars for each stage

## Quick Start

### Build

```bash
git clone <repo>
cd ehentai-crawler
cargo build --release
```

### Usage

```bash
# By gallery ID
./target/release/ehentai-crawler 623223

# By gallery URL
./target/release/ehentai-crawler "https://ehentai.to/g/623223"

# By viewer page URL
./target/release/ehentai-crawler "https://ehentai.to/g/623223/1/"
```

All three forms are equivalent. Output: `623223.cbz` in the current directory.

### Options

```
Usage: ehentai-crawler [OPTIONS] <URL>

Arguments:
  <URL>  Gallery ID or URL

Options:
  -o, --output <OUTPUT>          Output directory [default: .]
  -c, --concurrency <N>          Concurrent downloads [default: 4]
  -h, --help                     Print help
  -V, --version                  Print version
```

### Example

```bash
./target/release/ehentai-crawler 623223 -o ~/Downloads -c 8
```

```
Fetching gallery info from page 1...
Gallery ID: 623223  |  Total pages: 47
[00:00:10] [========================================] 47/47 pages (eta: 0s)
Downloading 47 images...
[00:00:06] [========================================] 47/47 images (eta: 0s)
Packing into /home/user/Downloads/623223.cbz...
Done! Saved to: /home/user/Downloads/623223.cbz
```

## Requirements

- Rust 1.75+
- OpenSSL (for TLS)

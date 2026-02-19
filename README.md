# ehentai-crawler

> [中文文档](README.zh.md)

A fast, async Rust CLI tool that downloads an entire gallery from [ehentai.to](https://ehentai.to), converts all images to PNG, and packages them into a `.cbz` file ready for any comic reader.

## Features

- Accepts gallery ID, gallery URL, or viewer page URL as input
- Automatically detects total page count
- **Pipeline architecture** — scraping, downloading, PNG conversion, and CBZ packing all run concurrently and overlap with each other, so no stage has to wait for the previous one to fully finish. Your urgent needs, met at full speed.
- Concurrent downloads and page scraping with configurable parallelism (defaults to all logical CPUs)
- Converts any image format (WebP, JPEG, GIF, etc.) to PNG using the blocking thread pool, saturating all CPU cores
- Four real-time progress bars, one per pipeline stage
- Interactive shell mode — run without arguments to download multiple galleries in one session

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
# By gallery ID
./target/release/ehentai-crawler 623223

# By gallery URL
./target/release/ehentai-crawler "https://ehentai.to/g/623223"

# By viewer page URL
./target/release/ehentai-crawler "https://ehentai.to/g/623223/1/"

# Interactive shell mode (no arguments)
./target/release/ehentai-crawler
```

All URL forms are equivalent. Output: `623223.cbz` in the current directory.

### Options

```
Usage: ehentai-crawler [OPTIONS] [URL]

Arguments:
  [URL]  Gallery ID or URL (omit to enter interactive mode)

Options:
  -o, --output <OUTPUT>          Output directory [default: .]
  -c, --concurrency <N>          Concurrent downloads [default: logical CPU count]
  -h, --help                     Print help
  -V, --version                  Print version
```

### Example

```bash
./target/release/ehentai-crawler 623223 -o ~/Downloads
```

```
Fetching gallery info...
Gallery ID: 623223  |  Total pages: 47
⠸ scraping  [=====================================>-] 46/47 (1s)
⠼ download  [=================================>-----] 40/47 (2s)
⠴ convert   [===========================>-----------] 34/47 (3s)
⠦ packing   [======================>----------------] 28/47 (3s)
Saved to: /home/user/Downloads/623223.cbz
```

## Requirements

- Rust 1.75+
- OpenSSL (for TLS)

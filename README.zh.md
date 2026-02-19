# ehentai-crawler

> [English README](README.md)

一个基于 Rust 异步编写的命令行工具，可自动爬取 [ehentai.to](https://ehentai.to) 上的完整漫画，将所有图片转换为 PNG 格式，并打包为 `.cbz` 文件，可直接用漫画阅读器打开。

## 功能特性

- 支持输入漫画 ID、漫画页 URL 或阅读器页面 URL
- 自动识别总页数，从第 1 页爬取到最后一页
- 并发抓取页面与下载图片，速度快
- 自动将任意格式图片（WebP、JPEG、GIF 等）转换为 PNG
- 输出 `.cbz` 压缩包，文件名零填充保证排序正确
- 每个阶段均有进度条显示

## 快速开始

### 编译

```bash
git clone <repo>
cd ehentai-crawler
cargo build --release
```

### 使用方式

```bash
# 直接输入漫画 ID
./target/release/ehentai-crawler 623223

# 输入漫画 URL
./target/release/ehentai-crawler "https://ehentai.to/g/623223"

# 输入阅读器页面 URL（任意页均可）
./target/release/ehentai-crawler "https://ehentai.to/g/623223/1/"
```

三种方式完全等价，输出文件为当前目录下的 `623223.cbz`。

### 参数说明

```
用法：ehentai-crawler [OPTIONS] <URL>

参数：
  <URL>  漫画 ID 或 URL

选项：
  -o, --output <OUTPUT>          输出目录 [默认: 当前目录]
  -c, --concurrency <N>          并发下载数 [默认: 4]
  -h, --help                     显示帮助
  -V, --version                  显示版本
```

### 示例

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

## 环境要求

- Rust 1.75+
- OpenSSL（用于 TLS）

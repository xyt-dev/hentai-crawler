# ehentai-crawler

> [English README](README.md)

一个基于 Rust 异步编写的命令行工具，可自动爬取 [ehentai.to](https://ehentai.to) 上的完整漫画，将所有图片转换为 PNG 格式，并打包为 `.cbz` 文件，可直接用漫画阅读器打开。

## 功能特性

- 支持输入漫画 ID、漫画页 URL 或阅读器页面 URL
- 自动识别总页数，从第 1 页爬取到最后一页
- **采用流水线技术优化** —— 抓取、下载、PNG 转换、CBZ 打包四个阶段同时运行、互相重叠，解决您的燃眉之急，绝不让您多等一秒
- 并发抓取与下载，默认并发数等于逻辑 CPU 核心数，充分压满带宽
- PNG 转换通过阻塞线程池并行执行，充分利用所有 CPU 核心
- 四条实时进度条，每个流水线阶段独立显示
- 交互式 Shell 模式 —— 无参数启动后可连续输入多个漫画 ID，批量下载

## 流水线架构

```
Stage 1 [scraping]  ████████░░░░░░░░░░░░░░░░░░
Stage 2 [download]    ░░░░████████████░░░░░░░░
Stage 3 [convert]         ░░░░░░░████████████░
Stage 4 [packing]               ░░░░░░░░░████
```

各阶段通过有界 channel 连接，找到图片 URL 立刻开始下载，下载完立刻开始转换，无需等待整个画廊的某一阶段全部完成。

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

# 交互式模式（无参数启动）
./target/release/ehentai-crawler
```

三种 URL 形式完全等价，输出文件为当前目录下的 `623223.cbz`。

### 参数说明

```
用法：ehentai-crawler [OPTIONS] [URL]

参数：
  [URL]  漫画 ID 或 URL（省略则进入交互式模式）

选项：
  -o, --output <OUTPUT>          输出目录 [默认: 当前目录]
  -c, --concurrency <N>          并发数 [默认: 逻辑 CPU 核心数]
  -h, --help                     显示帮助
  -V, --version                  显示版本
```

### 示例

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

## 环境要求

- Rust 1.75+
- OpenSSL（用于 TLS）

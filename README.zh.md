# hentai-crawler

> [English README](README.md)

一个基于 Rust 异步编写的命令行工具，采用 **四阶段流水线技术**，将抓取、下载、转换、打包同时并行推进，支持 [ehentai.to](https://ehentai.to) 和 [e-hentai.org](https://e-hentai.org)，自动爬取完整漫画并输出 `.cbz` 文件，可直接用漫画阅读器打开。

## 功能特性

- 同时支持 **ehentai.to** 和 **e-hentai.org** 两个站点
- 支持输入漫画 URL 或阅读器页面 URL
- 自动识别总页数，处理多页缩略图列表
- **采用流水线技术优化** —— 抓取、下载、PNG 转换、CBZ 打包四个阶段同时运行、互相重叠，绝不让您多等一秒
- 并发抓取与下载，默认并发数等于逻辑 CPU 核心数，充分压满带宽
- PNG 转换通过阻塞线程池并行执行，充分利用所有 CPU 核心
- 四条实时进度条，每个流水线阶段独立显示
- 交互式 Shell 模式 —— 无参数启动后可连续输入多个漫画 URL，批量下载
- 可选 Cookie 认证，用于 e-hentai.org 登录状态访问

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
# ehentai.to
./target/release/ehentai-crawler "https://ehentai.to/g/<id>"
./target/release/ehentai-crawler "https://ehentai.to/g/<id>/<page>/"

# e-hentai.org
./target/release/ehentai-crawler "https://e-hentai.org/g/<id>/<token>/"
./target/release/ehentai-crawler "https://e-hentai.org/g/<id>/<token>/<page>/"

# 交互式模式（无参数启动）
./target/release/ehentai-crawler
```

### 参数说明

```
用法：ehentai-crawler [OPTIONS] [URL]

参数：
  [URL]  漫画 URL（省略则进入交互式模式）

选项：
  -o, --output <OUTPUT>          输出目录 [默认: 当前目录]
  -c, --concurrency <N>          并发数 [默认: 逻辑 CPU 核心数]
      --cookies <COOKIES>        e-hentai.org 登录 Cookie 字符串
                                 例："ipb_member_id=123; ipb_pass_hash=abc; igneous=xyz"
  -h, --help                     显示帮助
  -V, --version                  显示版本
```

### 示例

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

## 支持的 URL 格式

| 站点 | 格式 |
|------|------|
| ehentai.to | `https://ehentai.to/g/<id>` |
| ehentai.to | `https://ehentai.to/g/<id>/<page>/` |
| e-hentai.org | `https://e-hentai.org/g/<id>/<token>/` |
| e-hentai.org | `https://e-hentai.org/g/<id>/<token>/<page>/` |

## 环境要求

- Rust 1.75+
- OpenSSL（用于 TLS）

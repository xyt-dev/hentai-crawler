use anyhow::{Context, Result, anyhow};
use clap::Parser;
use image::ImageFormat;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use std::collections::BTreeMap;
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::sync::Arc;
use zip::{ZipWriter, write::SimpleFileOptions};

/// ehentai.to manga crawler - downloads all pages and packs into a .cbz file
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Gallery ID or URL (e.g. 623223, https://ehentai.to/g/623223, https://ehentai.to/g/623223/1/)
    /// Omit to enter interactive shell mode.
    url: Option<String>,

    /// Output directory for the .cbz file
    #[arg(short, long, default_value = ".")]
    output: PathBuf,

    /// Number of concurrent downloads (default: number of logical CPUs)
    #[arg(short, long)]
    concurrency: Option<usize>,
}

fn parse_gallery_url(input: &str) -> Result<(String, String)> {
    let input = input.trim().trim_end_matches('/');

    if input.chars().all(|c| c.is_ascii_digit()) {
        return Ok(("https://ehentai.to".to_string(), input.to_string()));
    }

    let re_page = Regex::new(r"^(https?://[^/]+)/g/(\d+)/\d+$")?;
    if let Some(caps) = re_page.captures(input) {
        return Ok((caps[1].to_string(), caps[2].to_string()));
    }

    let re_gallery = Regex::new(r"^(https?://[^/]+)/g/(\d+)$")?;
    if let Some(caps) = re_gallery.captures(input) {
        return Ok((caps[1].to_string(), caps[2].to_string()));
    }

    Err(anyhow!(
        "Invalid input. Accepted formats:\n  623223\n  https://ehentai.to/g/623223\n  https://ehentai.to/g/623223/1/"
    ))
}

fn page_url(base: &str, gallery_id: &str, page: u32) -> String {
    format!("{}/g/{}/{}/", base, gallery_id, page)
}

async fn fetch_image_url(client: &Client, url: &str) -> Result<String> {
    let html = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to GET {url}"))?
        .text()
        .await?;

    let doc = Html::parse_document(&html);
    let sel = Selector::parse("#image-container img").unwrap();
    if let Some(el) = doc.select(&sel).next() {
        if let Some(src) = el.value().attr("src") {
            return Ok(src.to_string());
        }
    }

    if html.contains("num_pages") {
        anyhow::bail!("Found JS data but no <img> in #image-container on: {url}");
    }
    anyhow::bail!("Could not find image on: {url}")
}

fn extract_num_pages(html: &str) -> Result<u32> {
    let re = Regex::new(r#""num_pages"\s*:\s*(\d+)"#)?;
    let caps = re
        .captures(html)
        .ok_or_else(|| anyhow!("Could not find num_pages in page HTML"))?;
    Ok(caps[1].parse()?)
}

async fn download_image(client: &Client, url: &str) -> Result<bytes::Bytes> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to download image: {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} downloading {url}", resp.status());
    }
    Ok(resp.bytes().await?)
}

fn to_png(data: &[u8]) -> Result<Vec<u8>> {
    let format = image::guess_format(data).unwrap_or(ImageFormat::WebP);
    if format == ImageFormat::Png {
        return Ok(data.to_vec());
    }
    let img = image::load_from_memory(data).context("Failed to decode image")?;
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png)
        .context("Failed to encode PNG")?;
    Ok(buf)
}

fn make_pb(mp: &MultiProgress, total: u64, label: &str, color: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(total));
    pb.set_style(
        ProgressStyle::with_template(&format!(
            "{{spinner:.{color}}} {label:9} [{{bar:38.{color}/black}}] {{pos:>3}}/{{len}} ({{eta}})"
        ))
        .unwrap()
        .progress_chars("=>-"),
    );
    pb
}

/// Pipeline:
///   Stage 1 (crawl URLs)  ──► tx_url
///   Stage 2 (download)    ──► tx_raw  (bounded, backpressure)
///   Stage 3 (PNG convert) ──► tx_png  (bounded, backpressure)
///   Stage 4 (pack CBZ)    collects in BTreeMap then writes
async fn crawl(input: &str, output: &PathBuf, concurrency: usize, client: Arc<Client>) -> Result<()> {
    let (base, gallery_id) = parse_gallery_url(input)?;

    // ── Bootstrap: fetch page 1 to get num_pages ──────────────────────────
    println!("Fetching gallery info...");
    let p1_url = page_url(&base, &gallery_id, 1);
    let p1_html = client
        .get(&p1_url)
        .send()
        .await
        .with_context(|| format!("Failed to GET {p1_url}"))?
        .text()
        .await?;

    let num_pages = extract_num_pages(&p1_html)?;
    println!("Gallery ID: {gallery_id}  |  Total pages: {num_pages}");

    let p1_img_url = {
        let doc = Html::parse_document(&p1_html);
        let sel = Selector::parse("#image-container img").unwrap();
        doc.select(&sel)
            .next()
            .and_then(|el| el.value().attr("src").map(str::to_string))
            .ok_or_else(|| anyhow!("No image found on page 1"))?
    };

    let digits = num_pages.to_string().len();

    // ── Progress bars ──────────────────────────────────────────────────────
    let mp = Arc::new(MultiProgress::new());
    let pb_url = make_pb(&mp, num_pages as u64, "scraping", "cyan");
    let pb_dl  = make_pb(&mp, num_pages as u64, "download", "green");
    let pb_png = make_pb(&mp, num_pages as u64, "convert",  "yellow");
    let pb_cbz = make_pb(&mp, num_pages as u64, "packing",  "blue");

    // ── Channels ───────────────────────────────────────────────────────────
    // url_tx: (page, img_url)
    let (url_tx, mut url_rx) = tokio::sync::mpsc::channel::<(u32, String)>(concurrency * 2);
    // raw_tx: (page, raw_bytes)
    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::channel::<(u32, bytes::Bytes)>(concurrency * 2);
    // png_tx: (page, png_bytes)
    let (png_tx, mut png_rx) = tokio::sync::mpsc::channel::<(u32, Vec<u8>)>(concurrency * 2);

    // ── Stage 1: Scrape viewer pages → image URLs ──────────────────────────
    let stage1 = {
        let client = client.clone();
        let base = base.clone();
        let gallery_id = gallery_id.clone();
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let pb_url = pb_url.clone();

        tokio::spawn(async move {
            // Page 1 URL already known
            pb_url.inc(1);
            url_tx.send((1, p1_img_url)).await.ok();

            let mut handles = Vec::new();
            for page in 2..=num_pages {
                let url = page_url(&base, &gallery_id, page);
                let client = client.clone();
                let sem = sem.clone();
                let url_tx = url_tx.clone();
                let pb_url = pb_url.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    match fetch_image_url(&client, &url).await {
                        Ok(img_url) => {
                            pb_url.inc(1);
                            url_tx.send((page, img_url)).await.ok();
                        }
                        Err(e) => eprintln!("  [warn] page {page}: {e}"),
                    }
                }));
            }
            for h in handles { h.await.ok(); }
            // url_tx dropped here → url_rx will see None after last message
        })
    };

    // ── Stage 2: Download images ───────────────────────────────────────────
    let stage2 = {
        let client = client.clone();
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let pb_dl = pb_dl.clone();

        tokio::spawn(async move {
            let mut handles = Vec::new();
            while let Some((page, img_url)) = url_rx.recv().await {
                let client = client.clone();
                let sem = sem.clone();
                let raw_tx = raw_tx.clone();
                let pb_dl = pb_dl.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    match download_image(&client, &img_url).await {
                        Ok(data) => {
                            pb_dl.inc(1);
                            raw_tx.send((page, data)).await.ok();
                        }
                        Err(e) => eprintln!("  [warn] download page {page}: {e}"),
                    }
                }));
            }
            for h in handles { h.await.ok(); }
        })
    };

    // ── Stage 3: Convert to PNG (spawn_blocking → thread pool) ────────────
    let stage3 = tokio::spawn(async move {
        let mut handles = Vec::new();
        while let Some((page, raw)) = raw_rx.recv().await {
            let png_tx = png_tx.clone();
            let pb_png = pb_png.clone();

            handles.push(tokio::task::spawn_blocking(move || {
                match to_png(&raw) {
                    Ok(png) => {
                        pb_png.inc(1);
                        let _ = png_tx.blocking_send((page, png));
                    }
                    Err(e) => eprintln!("  [warn] PNG convert page {page}: {e}"),
                }
            }));
        }
        for h in handles { h.await.ok(); }
        pb_png.finish_with_message("done");
    });

    // ── Stage 4: Collect & write CBZ ──────────────────────────────────────
    let cbz_name = format!("{}.cbz", gallery_id);
    let cbz_path = output.join(&cbz_name);
    let file = std::fs::File::create(&cbz_path)
        .with_context(|| format!("Cannot create {}", cbz_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let zip_opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    // Collect all PNGs into a BTreeMap (sorted by page number automatically).
    let mut pngs: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    while let Some((page, png)) = png_rx.recv().await {
        pb_cbz.inc(1);
        pngs.insert(page, png);
    }

    // Wait for all pipeline stages to finish before writing.
    let (r1, r2, r3) = tokio::join!(stage1, stage2, stage3);
    r1.context("stage1 panicked")?;
    r2.context("stage2 panicked")?;
    r3.context("stage3 panicked")?;

    pb_url.finish_with_message("done");
    pb_dl.finish_with_message("done");

    // Write pages in order.
    for (page, png) in pngs {
        let entry_name = format!("{:0>width$}.png", page, width = digits);
        zip.start_file(&entry_name, zip_opts)?;
        zip.write_all(&png)?;
    }
    zip.finish()?;

    pb_cbz.finish_with_message("done");
    println!("Saved to: {}\n", cbz_path.display());

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let concurrency = args.concurrency.unwrap_or_else(num_cpus::get);

    let client = Arc::new(
        Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0")
            .build()?,
    );

    if let Some(url) = args.url {
        crawl(&url, &args.output, concurrency, client).await?;
    } else {
        println!("ehentai-crawler interactive mode");
        println!("Enter a gallery ID or URL to download. Type 'quit' or 'exit' to quit.\n");

        loop {
            print!("gallery> ");
            std::io::stdout().flush()?;

            let mut line = String::new();
            if std::io::stdin().read_line(&mut line)? == 0 {
                break;
            }

            let input = line.trim();
            if input.is_empty() {
                continue;
            }
            if input == "quit" || input == "exit" {
                println!("Bye!");
                break;
            }

            if let Err(e) = crawl(input, &args.output, concurrency, client.clone()).await {
                eprintln!("Error: {e:#}\n");
            }
        }
    }

    Ok(())
}

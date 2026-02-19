use anyhow::{Context, Result, anyhow};
use clap::Parser;
use image::ImageFormat;
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};
use std::io::Write as IoWrite;
use std::path::PathBuf;
use zip::{ZipWriter, write::SimpleFileOptions};

/// ehentai.to manga crawler - downloads all pages and packs into a .cbz file
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Gallery ID or URL (e.g. 623223, https://ehentai.to/g/623223, https://ehentai.to/g/623223/1/)
    url: String,

    /// Output directory for the .cbz file
    #[arg(short, long, default_value = ".")]
    output: PathBuf,

    /// Number of concurrent downloads
    #[arg(short, long, default_value_t = 4)]
    concurrency: usize,
}

/// Parse gallery ID and base URL from various input formats:
/// - "623223"
/// - "https://ehentai.to/g/623223"
/// - "https://ehentai.to/g/623223/"
/// - "https://ehentai.to/g/623223/1/"
fn parse_gallery_url(input: &str) -> Result<(String, String)> {
    let input = input.trim().trim_end_matches('/');

    // Pure numeric ID
    if input.chars().all(|c| c.is_ascii_digit()) {
        return Ok(("https://ehentai.to".to_string(), input.to_string()));
    }

    // URL with page: https://ehentai.to/g/623223/1
    let re_page = Regex::new(r"^(https?://[^/]+)/g/(\d+)/\d+$")?;
    if let Some(caps) = re_page.captures(input) {
        return Ok((caps[1].to_string(), caps[2].to_string()));
    }

    // URL without page: https://ehentai.to/g/623223
    let re_gallery = Regex::new(r"^(https?://[^/]+)/g/(\d+)$")?;
    if let Some(caps) = re_gallery.captures(input) {
        return Ok((caps[1].to_string(), caps[2].to_string()));
    }

    Err(anyhow!(
        "Invalid input. Accepted formats:\n  623223\n  https://ehentai.to/g/623223\n  https://ehentai.to/g/623223/1/"
    ))
}

/// Build a viewer page URL.
fn page_url(base: &str, gallery_id: &str, page: u32) -> String {
    format!("{}/g/{}/{}/", base, gallery_id, page)
}

/// Fetch a viewer page and extract the direct image src from #image-container img.
async fn fetch_image_url(client: &Client, url: &str) -> Result<String> {
    let html = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to GET {url}"))?
        .text()
        .await?;

    let doc = Html::parse_document(&html);

    // Try #image-container img first
    let sel = Selector::parse("#image-container img").unwrap();
    if let Some(el) = doc.select(&sel).next() {
        if let Some(src) = el.value().attr("src") {
            return Ok(src.to_string());
        }
    }

    // Fallback: search for num_pages in embedded JS to confirm we're on a valid page
    if html.contains("num_pages") {
        anyhow::bail!("Found JS data but no <img> in #image-container on page: {url}");
    }

    anyhow::bail!("Could not find image on page: {url}")
}

/// Extract num_pages from the embedded JS: `"num_pages": 47`
fn extract_num_pages(html: &str) -> Result<u32> {
    let re = Regex::new(r#""num_pages"\s*:\s*(\d+)"#)?;
    let caps = re
        .captures(html)
        .ok_or_else(|| anyhow!("Could not find num_pages in page HTML"))?;
    Ok(caps[1].parse()?)
}

/// Download image bytes from a URL.
async fn download_image(client: &Client, url: &str) -> Result<bytes::Bytes> {
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to download image: {url}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} when downloading {url}", resp.status());
    }

    Ok(resp.bytes().await?)
}

/// Convert raw image bytes to PNG bytes. Returns original bytes if already PNG.
fn to_png(data: &[u8]) -> Result<Vec<u8>> {
    // Detect format
    let format = image::guess_format(data).unwrap_or(ImageFormat::WebP);
    if format == ImageFormat::Png {
        return Ok(data.to_vec());
    }

    let img = image::load_from_memory(data)
        .with_context(|| "Failed to decode image")?;

    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png)
        .context("Failed to encode image as PNG")?;
    Ok(buf)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let (base, gallery_id) = parse_gallery_url(&args.url)?;

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0")
        .build()?;

    // --- Step 1: Fetch page 1 to get num_pages ---
    println!("Fetching gallery info from page 1...");
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

    // Extract image URL from page 1's HTML directly (avoid double fetch)
    let p1_img_url = {
        let doc = Html::parse_document(&p1_html);
        let sel = Selector::parse("#image-container img").unwrap();
        doc.select(&sel)
            .next()
            .and_then(|el| el.value().attr("src").map(str::to_string))
            .ok_or_else(|| anyhow!("No image found on page 1"))?
    };

    // --- Step 2: Collect all image URLs concurrently ---
    let pb = ProgressBar::new(num_pages as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} pages ({eta})",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    // We already have page 1 image URL; fetch the rest
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(args.concurrency));
    let client = std::sync::Arc::new(client);

    let mut tasks = Vec::new();

    // Page 1 is already resolved
    tasks.push(tokio::spawn({
        let pb = pb.clone();
        async move {
            pb.inc(1);
            Ok::<(u32, String), anyhow::Error>((1, p1_img_url))
        }
    }));

    for page in 2..=num_pages {
        let url = page_url(&base, &gallery_id, page);
        let client = client.clone();
        let sem = semaphore.clone();
        let pb = pb.clone();

        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let img_url = fetch_image_url(&client, &url).await?;
            pb.inc(1);
            Ok::<(u32, String), anyhow::Error>((page, img_url))
        }));
    }

    let mut page_img_urls: Vec<(u32, String)> = Vec::with_capacity(num_pages as usize);
    for task in tasks {
        let result = task.await.context("Task panicked")??;
        page_img_urls.push(result);
    }
    pb.finish_with_message("Page URLs collected");

    page_img_urls.sort_by_key(|(page, _)| *page);

    // --- Step 3: Download images concurrently ---
    println!("Downloading {} images...", num_pages);
    let pb2 = ProgressBar::new(num_pages as u64);
    pb2.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] [{bar:40.green/black}] {pos}/{len} images ({eta})",
        )
        .unwrap()
        .progress_chars("=>-"),
    );

    let semaphore2 = std::sync::Arc::new(tokio::sync::Semaphore::new(args.concurrency));

    let mut dl_tasks = Vec::new();
    for (page, img_url) in page_img_urls {
        let client = client.clone();
        let sem = semaphore2.clone();
        let pb = pb2.clone();

        dl_tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let data = download_image(&client, &img_url).await?;
            pb.inc(1);
            Ok::<(u32, bytes::Bytes), anyhow::Error>((page, data))
        }));
    }

    let mut images: Vec<(u32, bytes::Bytes)> = Vec::with_capacity(num_pages as usize);
    for task in dl_tasks {
        let result = task.await.context("Download task panicked")??;
        images.push(result);
    }
    pb2.finish_with_message("Images downloaded");

    images.sort_by_key(|(page, _)| *page);

    // --- Step 4: Convert to PNG and pack into .cbz ---
    let cbz_name = format!("{}.cbz", gallery_id);
    let cbz_path = args.output.join(&cbz_name);
    println!("Packing into {}...", cbz_path.display());

    let file = std::fs::File::create(&cbz_path)
        .with_context(|| format!("Cannot create {}", cbz_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let digits = num_pages.to_string().len();
    for (page, raw) in images {
        let png = to_png(&raw)?;
        let entry_name = format!("{:0>width$}.png", page, width = digits);
        zip.start_file(&entry_name, options)?;
        zip.write_all(&png)?;
    }

    zip.finish()?;
    println!("Done! Saved to: {}", cbz_path.display());

    Ok(())
}

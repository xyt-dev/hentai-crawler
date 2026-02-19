use anyhow::{Context, Result, anyhow};
use clap::Parser;
use image::ImageFormat;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::{Client, header};
use scraper::{Html, Selector};
use std::collections::BTreeMap;
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::sync::Arc;
use zip::{ZipWriter, write::SimpleFileOptions};

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Manga crawler supporting ehentai.to and e-hentai.org — downloads all pages and packs into a .cbz file
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Gallery URL.
    ///
    /// ehentai.to:
    ///   https://ehentai.to/g/<id>
    ///   https://ehentai.to/g/<id>/<page>/
    ///
    /// e-hentai.org:
    ///   https://e-hentai.org/g/<id>/<token>/
    ///   https://e-hentai.org/g/<id>/<token>/<page>/
    ///
    /// Omit to enter interactive shell mode.
    url: Option<String>,

    /// Output directory for the .cbz file
    #[arg(short, long, default_value = ".")]
    output: PathBuf,

    /// Number of concurrent downloads (default: number of logical CPUs)
    #[arg(short, long)]
    concurrency: Option<usize>,

    /// Cookies for e-hentai.org authentication (format: "name=value; name2=value2")
    /// Required for adult content on e-hentai.org. Typically needs:
    ///   ipb_member_id, ipb_pass_hash, igneous
    #[arg(long)]
    cookies: Option<String>,
}

// ── Site abstraction ──────────────────────────────────────────────────────────

enum Site {
    EhentaiTo { base: String, gallery_id: String },
    EHentaiOrg { gallery_id: String, token: String },
}

impl Site {
    fn parse(input: &str) -> Result<Site> {
        let input = input.trim().trim_end_matches('/');

        // e-hentai.org gallery page: https://e-hentai.org/g/ID/TOKEN[/PAGE]
        let re_ehorg = Regex::new(r"^https?://e-hentai\.org/g/(\d+)/([0-9a-f]+)(?:/\d+)?$")?;
        if let Some(caps) = re_ehorg.captures(input) {
            return Ok(Site::EHentaiOrg {
                gallery_id: caps[1].to_string(),
                token: caps[2].to_string(),
            });
        }

        // ehentai.to gallery: https://ehentai.to/g/ID[/PAGE]
        let re_page = Regex::new(r"^(https?://[^/]+)/g/(\d+)/\d+$")?;
        if let Some(caps) = re_page.captures(input) {
            return Ok(Site::EhentaiTo {
                base: caps[1].to_string(),
                gallery_id: caps[2].to_string(),
            });
        }
        let re_gallery = Regex::new(r"^(https?://[^/]+)/g/(\d+)$")?;
        if let Some(caps) = re_gallery.captures(input) {
            return Ok(Site::EhentaiTo {
                base: caps[1].to_string(),
                gallery_id: caps[2].to_string(),
            });
        }

        Err(anyhow!(
            "Invalid input. Accepted formats:\n\
             ehentai.to  : https://ehentai.to/g/<id>\n\
             e-hentai.org: https://e-hentai.org/g/<id>/<token>/"
        ))
    }
}

// ── ehentai.to helpers ────────────────────────────────────────────────────────

fn ehto_page_url(base: &str, gallery_id: &str, page: u32) -> String {
    format!("{}/g/{}/{}/", base, gallery_id, page)
}

async fn ehto_fetch_image_url(client: &Client, url: &str) -> Result<String> {
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

fn ehto_extract_num_pages(html: &str) -> Result<u32> {
    let re = Regex::new(r#""num_pages"\s*:\s*(\d+)"#)?;
    let caps = re
        .captures(html)
        .ok_or_else(|| anyhow!("Could not find num_pages in page HTML"))?;
    Ok(caps[1].parse()?)
}

fn ehto_extract_title(html: &str) -> Option<String> {
    // JS data contains: "title": { "english": "...", "japanese": "..." }
    let re = Regex::new(r#""title"\s*:\s*\{[^}]*"english"\s*:\s*"([^"]+)""#).ok()?;
    if let Some(caps) = re.captures(html) {
        return Some(caps[1].to_string());
    }
    // Fallback: japanese title
    let re_jp = Regex::new(r#""title"\s*:\s*\{[^}]*"japanese"\s*:\s*"([^"]+)""#).ok()?;
    re_jp.captures(html).map(|caps| caps[1].to_string())
}

// ── e-hentai.org helpers ──────────────────────────────────────────────────────

/// Information extracted from the e-hentai.org gallery page.
struct EHOrgGalleryInfo {
    title: String,
    num_pages: u32,
    /// Viewer page URLs for every page (sorted by page number, 1-indexed).
    viewer_urls: Vec<String>,
}

/// Fetch and parse the e-hentai.org gallery page.
/// e-hentai paginates the thumbnail list at 40 thumbnails per page;
/// we iterate all paginated gallery pages to collect every viewer URL.
async fn eho_fetch_gallery_info(
    client: &Client,
    gallery_id: &str,
    token: &str,
) -> Result<EHOrgGalleryInfo> {
    let base_url = format!("https://e-hentai.org/g/{}/{}/", gallery_id, token);

    // We collect viewer URLs page by page (40 per gallery listing page).
    let mut viewer_urls: Vec<String> = Vec::new();
    let mut num_pages: u32 = 0;
    let mut title = String::new();
    let mut listing_page = 0u32;

    loop {
        let url = if listing_page == 0 {
            base_url.clone()
        } else {
            format!("{}?p={}", base_url, listing_page)
        };

        let html = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Failed to GET {url}"))?
            .text()
            .await?;

        if listing_page == 0 {
            // Extract title
            let doc = Html::parse_document(&html);
            let title_sel = Selector::parse("#gn").unwrap();
            title = doc
                .select(&title_sel)
                .next()
                .map(|el| el.text().collect::<String>())
                .unwrap_or_else(|| gallery_id.to_string());

            // Extract total page count from "NNN pages" text
            num_pages = eho_extract_num_pages(&html)?;
        }

        // Collect viewer URLs from thumbnail links
        // Each thumbnail <a> has href like https://e-hentai.org/s/HASH/GALLERYID-PAGENUM
        let collected = eho_extract_viewer_urls(&html);
        if collected.is_empty() {
            break;
        }
        viewer_urls.extend(collected);

        // Check if there are more listing pages
        // e-hentai.org shows 20 thumbnails per gallery listing page
        const PER_PAGE: usize = 20;
        if viewer_urls.len() >= num_pages as usize {
            break;
        }
        // If we got fewer thumbnails than expected, we're done
        if viewer_urls.len() < (listing_page as usize + 1) * PER_PAGE {
            break;
        }
        listing_page += 1;
    }

    if viewer_urls.is_empty() {
        anyhow::bail!("No viewer URLs found on gallery page. The gallery may require login — use --cookies.");
    }

    Ok(EHOrgGalleryInfo {
        title,
        num_pages,
        viewer_urls,
    })
}

fn eho_extract_num_pages(html: &str) -> Result<u32> {
    // Matches: <td class="gdt2">123 pages</td>
    let re = Regex::new(r"(\d+)\s+pages?")?;
    if let Some(caps) = re.captures(html) {
        return Ok(caps[1].parse()?);
    }
    anyhow::bail!("Could not find page count on e-hentai.org gallery page")
}

fn eho_extract_viewer_urls(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    // Thumbnail links are direct <a> children of <div id="gdt">
    let sel = Selector::parse("#gdt a").unwrap();
    doc.select(&sel)
        .filter_map(|el| el.value().attr("href").map(str::to_string))
        .filter(|u| u.contains("e-hentai.org/s/"))
        .collect()
}

/// Fetch the actual image URL from an e-hentai.org viewer page.
/// The image is in `<img id="img" src="...">`.
async fn eho_fetch_image_url(client: &Client, viewer_url: &str) -> Result<String> {
    let html = client
        .get(viewer_url)
        .send()
        .await
        .with_context(|| format!("Failed to GET {viewer_url}"))?
        .text()
        .await?;

    // Parse the image src and optional nl-retry key synchronously,
    // then drop all DOM references before any .await so the future stays Send.
    let (maybe_img_src, maybe_nl_retry_url) = {
        let doc = Html::parse_document(&html);

        let img_src: Option<String> = {
            let sel = Selector::parse("#img").unwrap();
            doc.select(&sel)
                .next()
                .and_then(|el| el.value().attr("src"))
                .filter(|src| !src.contains("blank.") && !src.is_empty())
                .map(str::to_string)
        };

        let nl_retry_url: Option<String> = if img_src.is_none() {
            let nl_sel = Selector::parse("#loadfail").unwrap();
            doc.select(&nl_sel)
                .next()
                .and_then(|el| el.value().attr("onclick").map(str::to_string))
                .and_then(|onclick| {
                    let re = Regex::new(r"nl\('([^']+)'\)").unwrap();
                    re.captures(&onclick)
                        .map(|caps| caps[1].to_string())
                })
                .map(|nl_key| {
                    format!("{}?nl={}", viewer_url.trim_end_matches('/'), nl_key)
                })
        } else {
            None
        };

        (img_src, nl_retry_url)
        // `doc` dropped here — no non-Send values cross the await below
    };

    if let Some(src) = maybe_img_src {
        return Ok(src);
    }

    if let Some(retry_url) = maybe_nl_retry_url {
        return Box::pin(eho_fetch_image_url(client, &retry_url)).await;
    }

    anyhow::bail!("Could not find image on e-hentai.org viewer page: {viewer_url}")
}

// ── Shared pipeline helpers ───────────────────────────────────────────────────

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

// ── Pipeline ──────────────────────────────────────────────────────────────────

/// Pipeline:
///   Stage 1 (crawl URLs)  ──► tx_url
///   Stage 2 (download)    ──► tx_raw  (bounded, backpressure)
///   Stage 3 (PNG convert) ──► tx_png  (bounded, backpressure)
///   Stage 4 (pack CBZ)    collects in BTreeMap then writes
async fn crawl(input: &str, output: &PathBuf, concurrency: usize, client: Arc<Client>) -> Result<()> {
    let site = Site::parse(input)?;

    // ── Bootstrap: get num_pages and build the (page, viewer_url) list ────────
    println!("Fetching gallery info...");

    let (_gallery_id, num_pages, title, viewer_items): (String, u32, String, Vec<(u32, String)>) =
        match &site {
            Site::EhentaiTo { base, gallery_id } => {
                let p1_url = ehto_page_url(base, gallery_id, 1);
                let p1_html = client
                    .get(&p1_url)
                    .send()
                    .await
                    .with_context(|| format!("Failed to GET {p1_url}"))?
                    .text()
                    .await?;

                let num_pages = ehto_extract_num_pages(&p1_html)?;

                // Extract image URL for page 1 directly from the already-fetched HTML
                let p1_img_url = {
                    let doc = Html::parse_document(&p1_html);
                    let sel = Selector::parse("#image-container img").unwrap();
                    doc.select(&sel)
                        .next()
                        .and_then(|el| el.value().attr("src").map(str::to_string))
                        .ok_or_else(|| anyhow!("No image found on page 1"))?
                };

                // For ehentai.to, "viewer_url" for page 1 is already the image URL.
                // For pages 2..N we store the viewer page URL and resolve later.
                // To unify, we use a tagged enum per-item — but since the two sites
                // differ in what stage-1 does, we carry the *viewer* URL for all pages
                // except page 1 where we already have the image URL.
                // We encode this by carrying viewer URLs for all pages; for page 1 we
                // store the image URL directly under a sentinel.
                let mut items: Vec<(u32, String)> = vec![(1, p1_img_url)];
                for page in 2..=num_pages {
                    items.push((page, ehto_page_url(base, gallery_id, page)));
                }
                let title = ehto_extract_title(&p1_html)
                    .unwrap_or_else(|| gallery_id.clone());
                (gallery_id.clone(), num_pages, title, items)
            }

            Site::EHentaiOrg { gallery_id, token } => {
                let info = eho_fetch_gallery_info(&client, gallery_id, token).await?;
                let num_pages = info.num_pages;
                let items: Vec<(u32, String)> = info
                    .viewer_urls
                    .into_iter()
                    .enumerate()
                    .map(|(i, u)| (i as u32 + 1, u))
                    .collect();
                (gallery_id.clone(), num_pages, info.title, items)
            }
        };

    println!("Gallery: {title}  |  Total pages: {num_pages}");

    let digits = num_pages.to_string().len();

    // ── Progress bars ──────────────────────────────────────────────────────────
    let mp = Arc::new(MultiProgress::new());
    let pb_url = make_pb(&mp, num_pages as u64, "scraping", "cyan");
    let pb_dl  = make_pb(&mp, num_pages as u64, "download", "green");
    let pb_png = make_pb(&mp, num_pages as u64, "convert",  "yellow");
    let pb_cbz = make_pb(&mp, num_pages as u64, "packing",  "blue");

    // ── Channels ───────────────────────────────────────────────────────────────
    let (url_tx, mut url_rx) = tokio::sync::mpsc::channel::<(u32, String)>(concurrency * 2);
    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::channel::<(u32, bytes::Bytes)>(concurrency * 2);
    let (png_tx, mut png_rx) = tokio::sync::mpsc::channel::<(u32, Vec<u8>)>(concurrency * 2);

    // ── Stage 1: Resolve viewer pages → image URLs ─────────────────────────────
    //
    // For ehentai.to:  page 1 item already carries the image URL (no HTTP needed).
    //                  Pages 2..N carry the viewer-page URL → need fetch_image_url.
    // For e-hentai.org: all items carry a viewer-page URL → need eho_fetch_image_url.
    //
    // We distinguish them by site type captured via a boolean flag.
    let is_ehorg = matches!(site, Site::EHentaiOrg { .. });

    let stage1 = {
        let client = client.clone();
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let pb_url = pb_url.clone();

        tokio::spawn(async move {
            let mut handles = Vec::new();
            for (page, viewer_url) in viewer_items {
                let client = client.clone();
                let sem = sem.clone();
                let url_tx = url_tx.clone();
                let pb_url = pb_url.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();

                    // For ehentai.to page 1 the item IS the image URL already.
                    // Detect by checking: if not ehorg AND page==1, the viewer_url
                    // is actually an image URL (starts with http and is not a viewer page pattern).
                    // Simpler: we always call the fetch function; for ehto page 1 the
                    // "viewer_url" is the direct image URL, so we just forward it.
                    // But calling fetch on a direct image URL would return wrong content.
                    // Solution: use a sentinel prefix to mark already-resolved URLs.
                    let img_url_result = if !is_ehorg && page == 1 {
                        // Already an image URL — skip HTTP fetch
                        Ok(viewer_url)
                    } else if is_ehorg {
                        eho_fetch_image_url(&client, &viewer_url).await
                    } else {
                        ehto_fetch_image_url(&client, &viewer_url).await
                    };

                    match img_url_result {
                        Ok(img_url) => {
                            pb_url.inc(1);
                            url_tx.send((page, img_url)).await.ok();
                        }
                        Err(e) => eprintln!("  [warn] page {page}: {e}"),
                    }
                }));
            }
            for h in handles { h.await.ok(); }
        })
    };

    // ── Stage 2: Download images ───────────────────────────────────────────────
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

    // ── Stage 3: Convert to PNG ────────────────────────────────────────────────
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

    // ── Stage 4: Collect & write CBZ ──────────────────────────────────────────
    // Sanitize title for use as a filename: replace characters not allowed on
    // common filesystems (Windows/Linux) with underscores.
    let safe_title: String = title.chars().map(|c| match c {
        '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
        c => c,
    }).collect();
    let cbz_name = format!("{}.cbz", safe_title);
    let cbz_path = output.join(&cbz_name);
    let file = std::fs::File::create(&cbz_path)
        .with_context(|| format!("Cannot create {}", cbz_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let zip_opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let mut pngs: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    while let Some((page, png)) = png_rx.recv().await {
        pb_cbz.inc(1);
        pngs.insert(page, png);
    }

    let (r1, r2, r3) = tokio::join!(stage1, stage2, stage3);
    r1.context("stage1 panicked")?;
    r2.context("stage2 panicked")?;
    r3.context("stage3 panicked")?;

    pb_url.finish_with_message("done");
    pb_dl.finish_with_message("done");

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

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let concurrency = args.concurrency.unwrap_or_else(num_cpus::get);

    let mut client_builder = Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:124.0) Gecko/20100101 Firefox/124.0");

    if let Some(ref cookies) = args.cookies {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::COOKIE,
            header::HeaderValue::from_str(cookies)
                .context("Invalid cookie string")?,
        );
        client_builder = client_builder.default_headers(headers);
    }

    let client = Arc::new(client_builder.build()?);

    if let Some(url) = args.url {
        crawl(&url, &args.output, concurrency, client).await?;
    } else {
        println!("ehentai-crawler interactive mode");
        println!("Supported sites: ehentai.to, e-hentai.org");
        println!("Enter a gallery URL to download. Type 'quit' or 'exit' to quit.\n");

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

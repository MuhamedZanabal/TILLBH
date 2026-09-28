//! Product image worker (feature `catalog.auto_images`).
//!
//! Runs on the hub (or a standalone till), never on a terminal: terminals get
//! the stored image with the product rows. One product at a time, a few
//! seconds apart: claim it (the core's conditional update), search, download
//! the best candidates through the SSRF-guarded fetcher, validate and
//! normalize them in the core, keep the best one or none, and report the
//! outcome. A lookup runs once per product; failures never touch selling.
//!
//! Sources (see `ImageSearchSettings`):
//! * Open Food Facts: exact barcode lookup (no key). The strongest evidence.
//! * Bing image results page (no key), the approach of `bing-image-urls`:
//!   barcode + name, large white product photographs, configured market.
//! * Google Programmable Search, image search: optional, needs a key (secret
//!   store) and an engine id; asked for white-background product photos in the
//!   configured market (`gl` / `cr`), barcode + name first.
//!
//! Nothing here runs on a product read: screens only ever see stored images.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use amwapos_core::product_images::{normalize, AutoImageJob, AutoOutcome, ImageSearchSettings, Normalized, MAX_SOURCE_BYTES};
use amwapos_core::{AppCore, AppError, AppResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Pause between two lookups (provider rate limits; never a burst).
const BETWEEN_JOBS: Duration = Duration::from_secs(3);
/// Idle poll when nothing is queued.
const IDLE: Duration = Duration::from_secs(30);
/// Whole lookup budget for one product.
const JOB_TIMEOUT: Duration = Duration::from_secs(90);
/// Candidates downloaded and checked per product at most.
const MAX_DOWNLOADS: usize = 4;

/// A search hit before download.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub url: String,
    pub title: String,
    pub page_url: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The source ties this image to the exact barcode.
    pub barcode_match: bool,
    pub provider: &'static str,
}

#[derive(Debug, Clone)]
pub struct SearchError {
    /// Worth retrying later (network, 5xx, quota).
    pub transient: bool,
    pub message: String,
}

impl SearchError {
    fn transient(m: impl Into<String>) -> Self {
        Self { transient: true, message: m.into() }
    }
    fn permanent(m: impl Into<String>) -> Self {
        Self { transient: false, message: m.into() }
    }
}

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub barcode: Option<String>,
    pub name: String,
    pub name_ar: Option<String>,
    pub category: Option<String>,
    /// The words sent to a web search: "<barcode> <name>", or name + category.
    pub text: String,
    pub region: String,
    pub language: String,
}

/// A source of candidate product images. Provider code stays behind this.
#[async_trait]
pub trait ImageSearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(&self, q: &SearchQuery) -> Result<Vec<Candidate>, SearchError>;
}

/// A barcode worth looking up: 8–14 digits (EAN-8, UPC-A, EAN-13, GTIN-14).
pub fn usable_barcode(barcodes: &[String]) -> Option<String> {
    barcodes.iter().find(|b| (8..=14).contains(&b.len()) && b.chars().all(|c| c.is_ascii_digit())).cloned()
}

pub fn build_query(job: &AutoImageJob, cfg: &ImageSearchSettings) -> SearchQuery {
    let barcode = usable_barcode(&job.barcodes);
    let text = match &barcode {
        Some(b) => format!("{b} {}", job.name),
        None => match &job.category {
            Some(c) => format!("{} {c}", job.name),
            None => job.name.clone(),
        },
    };
    SearchQuery {
        barcode,
        name: job.name.clone(),
        name_ar: job.name_ar.clone(),
        category: job.category.clone(),
        text,
        region: cfg.region.clone(),
        language: cfg.language.clone(),
    }
}

// ---------------------------------------------------------------- providers

pub struct OpenFoodFacts {
    pub base: String,
    http: reqwest::Client,
}

impl OpenFoodFacts {
    pub fn new(base: &str) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), http: api_client() }
    }
}

#[async_trait]
impl ImageSearchProvider for OpenFoodFacts {
    fn name(&self) -> &'static str {
        "open_food_facts"
    }
    async fn search(&self, q: &SearchQuery) -> Result<Vec<Candidate>, SearchError> {
        let Some(code) = &q.barcode else { return Ok(vec![]) };
        let url = format!("{}/api/v2/product/{code}?fields=code,product_name,brands,image_front_url,image_url", self.base);
        let resp = self.http.get(&url).send().await.map_err(|e| SearchError::transient(net(&e)))?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok(vec![]);
        }
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(SearchError::transient(format!("Open Food Facts answered {status}")));
        }
        if !status.is_success() {
            return Err(SearchError::permanent(format!("Open Food Facts answered {status}")));
        }
        let v: Value = resp.json().await.map_err(|_| SearchError::transient("Open Food Facts sent an unreadable reply"))?;
        if v.get("status").and_then(|s| s.as_i64()) != Some(1) {
            return Ok(vec![]);
        }
        let p = &v["product"];
        let same_code = v.get("code").and_then(|c| c.as_str()) == Some(code.as_str());
        let title = [p["product_name"].as_str(), p["brands"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let mut out = vec![];
        for key in ["image_front_url", "image_url"] {
            if let Some(u) = p[key].as_str().filter(|u| !u.is_empty()) {
                if out.iter().any(|c: &Candidate| c.url == u) {
                    continue;
                }
                out.push(Candidate {
                    url: u.to_string(),
                    title: title.clone(),
                    page_url: Some(format!("https://world.openfoodfacts.org/product/{code}")),
                    width: None,
                    height: None,
                    barcode_match: same_code,
                    provider: "open_food_facts",
                });
            }
        }
        Ok(out)
    }
}

/// Bing image results, no key: the public results fragment
/// (`/images/async`) that the `bing-image-urls` package reads. Each result
/// carries an HTML-escaped JSON `m` attribute with `murl` (full image),
/// `purl` (page) and `t` (title). Filtered to large photographs, white,
/// in the configured market. Unofficial: the page may change or be limited,
/// so errors are transient and other sources still run.
pub struct BingImages {
    pub base: String,
    http: reqwest::Client,
}

impl BingImages {
    pub fn new(base: &str) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), http: api_client() }
    }
}

fn html_unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Parse the results fragment: every `m="{...}"` attribute with a `murl`.
pub fn parse_bing(html: &str, barcode: Option<&str>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = vec![];
    for part in html.split(" m=\"").skip(1) {
        let Some(end) = part.find('"') else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&html_unescape(&part[..end])) else { continue };
        let Some(murl) = v.get("murl").and_then(|u| u.as_str()) else { continue };
        if out.iter().any(|c| c.url == murl) {
            continue;
        }
        let title = v.get("t").and_then(|t| t.as_str()).unwrap_or("");
        let desc = v.get("desc").and_then(|t| t.as_str()).unwrap_or("");
        let page = v.get("purl").and_then(|t| t.as_str()).map(String::from);
        let text = format!("{title} {desc} {murl}");
        out.push(Candidate {
            barcode_match: barcode.is_some_and(|b| text.contains(b) || page.as_deref().is_some_and(|p| p.contains(b))),
            title: text,
            page_url: page,
            width: None,
            height: None,
            url: murl.to_string(),
            provider: "bing_images",
        });
        if out.len() >= 30 {
            break;
        }
    }
    out
}

#[async_trait]
impl ImageSearchProvider for BingImages {
    fn name(&self) -> &'static str {
        "bing_images"
    }
    async fn search(&self, q: &SearchQuery) -> Result<Vec<Candidate>, SearchError> {
        let cc = q.region.to_ascii_uppercase();
        let mkt = format!("{}-{cc}", q.language);
        let params = [
            ("q", q.text.as_str()),
            ("first", "0"),
            ("count", "35"),
            ("adlt", "strict"),
            ("qft", "+filterui:photo-photo+filterui:imagesize-large+filterui:color2-FGcls_WHITE"),
            ("cc", cc.as_str()),
            ("setmkt", mkt.as_str()),
            ("setlang", q.language.as_str()),
        ];
        let resp = self
            .http
            .get(format!("{}/images/async", self.base))
            .query(&params)
            .header(reqwest::header::ACCEPT_LANGUAGE, format!("{mkt},{};q=0.8,ar;q=0.5", q.language))
            .send()
            .await
            .map_err(|e| SearchError::transient(net(&e)))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SearchError::transient(format!("Bing images answered {status}")));
        }
        let body = resp.text().await.map_err(|_| SearchError::transient("Bing images sent an unreadable reply"))?;
        Ok(parse_bing(&body, q.barcode.as_deref()))
    }
}

pub struct GoogleImages {
    pub base: String,
    key: String,
    cx: String,
    http: reqwest::Client,
}

impl GoogleImages {
    pub fn new(base: &str, key: String, cx: String) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), key, cx, http: api_client() }
    }
}

#[async_trait]
impl ImageSearchProvider for GoogleImages {
    fn name(&self) -> &'static str {
        "google_images"
    }
    async fn search(&self, q: &SearchQuery) -> Result<Vec<Candidate>, SearchError> {
        let region = q.region.to_ascii_lowercase();
        let cr = format!("country{}", region.to_ascii_uppercase());
        let params = [
            ("key", self.key.as_str()),
            ("cx", self.cx.as_str()),
            ("q", q.text.as_str()),
            ("searchType", "image"),
            ("num", "10"),
            ("imgSize", "large"),
            ("imgType", "photo"),
            ("imgDominantColor", "white"),
            ("safe", "active"),
            ("gl", region.as_str()),
            ("cr", cr.as_str()),
            ("hl", q.language.as_str()),
        ];
        let resp = self
            .http
            .get(format!("{}/customsearch/v1", self.base))
            .query(&params)
            .send()
            .await
            .map_err(|e| SearchError::transient(net(&e)))?;
        let status = resp.status();
        if !status.is_success() {
            // 429 / 403 quota and 5xx are retried; the key never appears in messages.
            return Err(SearchError::transient(format!("web image search answered {status}")));
        }
        let v: Value = resp.json().await.map_err(|_| SearchError::transient("web image search sent an unreadable reply"))?;
        let items = v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|it| {
                let url = it.get("link")?.as_str()?.to_string();
                let img = it.get("image");
                let text = format!(
                    "{} {} {}",
                    it.get("title").and_then(|t| t.as_str()).unwrap_or(""),
                    it.get("snippet").and_then(|t| t.as_str()).unwrap_or(""),
                    url
                );
                Some(Candidate {
                    barcode_match: q.barcode.as_deref().is_some_and(|b| text.contains(b)),
                    title: text,
                    page_url: img.and_then(|i| i.get("contextLink")).and_then(|x| x.as_str()).map(String::from),
                    width: img.and_then(|i| i.get("width")).and_then(|x| x.as_u64()).map(|x| x as u32),
                    height: img.and_then(|i| i.get("height")).and_then(|x| x.as_u64()).map(|x| x as u32),
                    url,
                    provider: "google_images",
                })
            })
            .collect())
    }
}

fn api_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(6))
        .user_agent("AMWAPOS/0.1 (product image lookup)")
        .build()
        .unwrap_or_default()
}

/// A network error without the request URL (which may carry an API key).
fn net(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "the request timed out".into()
    } else if e.is_connect() {
        "could not connect".into()
    } else {
        "network error".into()
    }
}

// ---------------------------------------------------------------- scoring

const REJECT_WORDS: &[&str] =
    &["logo", "banner", "icon", "vector", "clipart", "clip art", "wallpaper", "illustration", "cartoon", "recipe", "poster", "advert"];
const GCC_TLDS: &[&str] = &[".bh", ".ae", ".sa", ".kw", ".qa", ".om"];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|t| t.chars().count() >= 3).map(String::from).collect()
}

/// Text relevance of a hit before download, or None to skip it. Without the
/// exact barcode, at least half of the product-name words must appear.
pub fn text_score(c: &Candidate, q: &SearchQuery) -> Option<f32> {
    let hay = format!("{} {}", c.title, c.page_url.as_deref().unwrap_or("")).to_lowercase();
    if REJECT_WORDS.iter().any(|w| hay.contains(w)) {
        return None;
    }
    if let (Some(w), Some(h)) = (c.width, c.height) {
        if w < 200 || h < 200 {
            return None;
        }
    }
    let name = tokens(&q.name);
    let overlap = if name.is_empty() { 0.0 } else { name.iter().filter(|t| hay.contains(t.as_str())).count() as f32 / name.len() as f32 };
    if !c.barcode_match && overlap < 0.5 {
        return None;
    }
    let mut s = if c.barcode_match { 3.0 } else { 0.0 } + 2.0 * overlap;
    if let Some(page) = &c.page_url {
        if let Ok(u) = reqwest::Url::parse(page) {
            if u.host_str().is_some_and(|h| GCC_TLDS.iter().any(|t| h.ends_with(t))) {
                s += 0.3;
            }
        }
    }
    Some(s)
}

/// Final score after the image was downloaded and decoded, or None when it
/// is not a usable product picture. Web hits must look like a packshot (a
/// mostly white frame); an exact-barcode source is trusted on identity.
pub fn image_score(c: &Candidate, text: f32, n: &Normalized) -> Option<f32> {
    let side = n.source_width.min(n.source_height);
    if side < 200 {
        return None;
    }
    let aspect = n.source_width.max(n.source_height) as f32 / side as f32;
    if aspect > 3.0 {
        return None; // banners and strips
    }
    if !c.barcode_match && n.white_border < 0.45 {
        return None;
    }
    Some(text + 2.0 * n.white_border + (side.min(1000) as f32 / 1000.0))
}

// ---------------------------------------------------------------- fetching

#[derive(Debug, Clone)]
pub struct FetchError {
    pub transient: bool,
    pub message: String,
}

fn fe(transient: bool, m: impl Into<String>) -> FetchError {
    FetchError { transient, message: m.into() }
}

/// Downloads an untrusted image URL safely: http(s) only on the standard
/// ports, no credentials in the URL, every address the name resolves to must
/// be public (no loopback, private, link-local, metadata, CGNAT, multicast,
/// unique-local or mapped-private addresses), the connection is pinned to the
/// checked address, redirects are followed by hand (at most 3, each checked
/// again), with timeouts, an image content type and a hard size cap.
pub struct SafeFetcher {
    allow_private: bool,
    max_bytes: usize,
}

impl Default for SafeFetcher {
    fn default() -> Self {
        Self { allow_private: false, max_bytes: MAX_SOURCE_BYTES }
    }
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // link local fe80::/10
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x0064 && s[1] == 0xff9b)) // NAT64 (may reach private v4)
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local() // 169.254/16, incl. 169.254.169.254 metadata
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF 192.0.0/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18/15
        || o[0] >= 240) // reserved
}

impl SafeFetcher {
    /// Tests only: allow a local fixture server.
    pub fn allowing_private_for_tests() -> Self {
        Self { allow_private: true, max_bytes: MAX_SOURCE_BYTES }
    }

    /// Check a URL and resolve it to one allowed address.
    pub async fn check(&self, raw: &str) -> Result<(reqwest::Url, SocketAddr), FetchError> {
        let url = reqwest::Url::parse(raw).map_err(|_| fe(false, "not a valid address"))?;
        if url.scheme() != "https" && url.scheme() != "http" {
            return Err(fe(false, "only http and https addresses are allowed"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(fe(false, "addresses with credentials are not allowed"));
        }
        let host = url.host_str().ok_or_else(|| fe(false, "the address has no host"))?.to_string();
        let port = url.port_or_known_default().ok_or_else(|| fe(false, "no port"))?;
        if !self.allow_private && url.port().is_some_and(|p| p != 80 && p != 443) {
            return Err(fe(false, "only the standard web ports are allowed"));
        }
        let lower = host.to_ascii_lowercase();
        if !self.allow_private
            && (lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") || lower.ends_with(".internal"))
        {
            return Err(fe(false, "internal host names are not allowed"));
        }
        let bare = lower.trim_start_matches('[').trim_end_matches(']').to_string();
        let addrs: Vec<SocketAddr> = match bare.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, port)],
            Err(_) => tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((bare.as_str(), port)))
                .await
                .map_err(|_| fe(true, "name lookup timed out"))?
                .map_err(|_| fe(true, "name lookup failed"))?
                .collect(),
        };
        if addrs.is_empty() {
            return Err(fe(true, "the name did not resolve"));
        }
        if !self.allow_private && addrs.iter().any(|a| !is_public_ip(a.ip())) {
            return Err(fe(false, "the address points to a private or internal network"));
        }
        Ok((url, addrs[0]))
    }

    pub async fn fetch(&self, raw: &str) -> Result<Vec<u8>, FetchError> {
        let mut next = raw.to_string();
        for _ in 0..=3 {
            let (url, addr) = self.check(&next).await?;
            let host = url.host_str().unwrap_or_default().to_string();
            let mut b = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .connect_timeout(Duration::from_secs(6))
                .user_agent("AMWAPOS/0.1 (product image lookup)");
            if host.parse::<IpAddr>().is_err() {
                b = b.resolve(&host, addr);
            }
            if self.allow_private {
                b = b.no_proxy();
            }
            let client = b.build().map_err(|_| fe(true, "could not prepare the download"))?;
            let resp = client.get(url.clone()).header(reqwest::header::ACCEPT, "image/*").send().await.map_err(|e| fe(true, net(&e)))?;
            let status = resp.status();
            if status.is_redirection() {
                let loc = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| fe(false, "a redirect without a destination"))?;
                next = url.join(loc).map_err(|_| fe(false, "a bad redirect"))?.to_string();
                continue;
            }
            if status.as_u16() == 429 || status.is_server_error() {
                return Err(fe(true, format!("the image server answered {status}")));
            }
            if !status.is_success() {
                return Err(fe(false, format!("the image server answered {status}")));
            }
            let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
            if !ctype.starts_with("image/") {
                return Err(fe(false, "the address is not an image"));
            }
            if resp.content_length().is_some_and(|n| n as usize > self.max_bytes) {
                return Err(fe(false, "the image is too large"));
            }
            let mut resp = resp;
            let mut body: Vec<u8> = Vec::new();
            while let Some(chunk) = resp.chunk().await.map_err(|e| fe(true, net(&e)))? {
                if body.len() + chunk.len() > self.max_bytes {
                    return Err(fe(false, "the image is too large"));
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(body);
        }
        Err(fe(false, "too many redirects"))
    }
}

// ---------------------------------------------------------------- lookup

/// Search every provider, download the best candidates, keep the best image.
pub async fn lookup(providers: &[Arc<dyn ImageSearchProvider>], fetcher: &SafeFetcher, q: &SearchQuery) -> AutoOutcome {
    let mut candidates: Vec<(f32, Candidate)> = vec![];
    let mut errors: Vec<String> = vec![];
    let mut transient = false;
    for p in providers {
        match p.search(q).await {
            Ok(hits) => {
                for c in hits {
                    if let Some(s) = text_score(&c, q) {
                        candidates.push((s, c));
                    }
                }
            }
            Err(e) => {
                transient |= e.transient;
                errors.push(format!("{}: {}", p.name(), e.message));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut best: Option<(f32, Normalized, Candidate)> = None;
    let mut rejected = 0usize;
    for (ts, c) in candidates.into_iter().take(MAX_DOWNLOADS) {
        let bytes = match fetcher.fetch(&c.url).await {
            Ok(b) => b,
            Err(e) => {
                rejected += 1;
                tracing::debug!(provider = c.provider, error = %e.message, "candidate image not downloaded");
                continue;
            }
        };
        let n = match tokio::task::spawn_blocking(move || normalize(&bytes)).await {
            Ok(Ok(n)) => n,
            _ => {
                rejected += 1;
                continue;
            }
        };
        match image_score(&c, ts, &n) {
            Some(s) if best.as_ref().is_none_or(|b| s > b.0) => best = Some((s, n, c)),
            Some(_) => {}
            None => rejected += 1,
        }
    }
    match best {
        Some((score, image, c)) => AutoOutcome::Found {
            note: json!({
                "provider": c.provider, "source_url": c.url, "source_page": c.page_url, "barcode_match": c.barcode_match,
                "score": (score * 100.0).round() / 100.0, "white_background": (image.white_border * 100.0).round() / 100.0,
                "query": q.text, "found_at": amwapos_core::time::now_str(),
            }),
            image,
        },
        None if transient => AutoOutcome::Transient { note: json!({ "errors": errors, "query": q.text }) },
        None => AutoOutcome::NotFound {
            note: json!({ "reason": "no confident match", "rejected": rejected, "errors": errors, "query": q.text }),
        },
    }
}

/// Which sources can run with the current settings (none: the worker idles
/// without claiming anything, so nothing is marked failed for lack of setup).
pub fn providers_for(cfg: &ImageSearchSettings, google_key: Option<String>) -> Vec<Arc<dyn ImageSearchProvider>> {
    let mut v: Vec<Arc<dyn ImageSearchProvider>> = vec![];
    let off_base = std::env::var("AMWAPOS_OFF_BASE").unwrap_or_else(|_| "https://world.openfoodfacts.org".into());
    let google_base = std::env::var("AMWAPOS_GOOGLE_SEARCH_BASE").unwrap_or_else(|_| "https://www.googleapis.com".into());
    if std::env::var("AMWAPOS_IMAGE_SEARCH").as_deref() == Ok("off") {
        return v;
    }
    if cfg.open_food_facts {
        v.push(Arc::new(OpenFoodFacts::new(&off_base)));
    }
    if cfg.bing {
        let bing_base = std::env::var("AMWAPOS_BING_BASE").unwrap_or_else(|_| "https://www.bing.com".into());
        v.push(Arc::new(BingImages::new(&bing_base)));
    }
    if cfg.google && !cfg.google_cx.is_empty() {
        if let Some(k) = google_key {
            v.push(Arc::new(GoogleImages::new(&google_base, k, cfg.google_cx.clone())));
        }
    }
    v
}

pub struct ImageWorker {
    core: Arc<AppCore>,
    task: Mutex<Option<JoinHandle<()>>>,
    pub poke: Arc<Notify>,
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::internal(format!("worker failed: {e}")))?
}

fn may_run(core: &AppCore) -> bool {
    let on = core.features().map(|f| f.is_on("catalog.auto_images")).unwrap_or(false);
    // Terminals receive products (and their stored images) from the hub.
    on && core.device().is_some_and(|d| d.mode != "terminal")
}

impl ImageWorker {
    pub fn new(core: Arc<AppCore>) -> Arc<Self> {
        Arc::new(Self { core, task: Mutex::new(None), poke: Arc::new(Notify::new()) })
    }

    /// Start the worker when the feature is on. Idempotent; restarts a dead task.
    pub fn ensure(self: &Arc<Self>) {
        let mut g = self.task.lock().unwrap();
        let alive = g.as_ref().map(|h| !h.is_finished()).unwrap_or(false);
        if may_run(&self.core) && !alive {
            *g = Some(tokio::spawn(run(self.clone())));
        }
    }

    pub fn running(&self) -> bool {
        self.task.lock().unwrap().as_ref().map(|h| !h.is_finished()).unwrap_or(false)
    }
}

/// One claimed product, start to finish. Returns false when nothing was due.
pub async fn process_one(core: &Arc<AppCore>, providers: &[Arc<dyn ImageSearchProvider>], fetcher: &SafeFetcher) -> AppResult<bool> {
    let (c, cfg) = (core.clone(), core.image_search_config()?.0);
    let Some(job) = blocking(move || c.auto_image_claim()).await? else { return Ok(false) };
    let q = build_query(&job, &cfg);
    let outcome = match tokio::time::timeout(JOB_TIMEOUT, lookup(providers, fetcher, &q)).await {
        Ok(o) => o,
        Err(_) => AutoOutcome::Transient { note: json!({ "errors": ["the lookup took too long"], "query": q.text }) },
    };
    let (c, pid) = (core.clone(), job.product_id.clone());
    let state = blocking(move || c.auto_image_complete(&pid, outcome)).await?;
    tracing::info!(product_id = %job.product_id, attempt = job.attempt, state = %state, "product image lookup");
    Ok(true)
}

async fn run(w: Arc<ImageWorker>) {
    let fetcher = SafeFetcher::default();
    loop {
        let c = w.core.clone();
        if !blocking(move || Ok(may_run(&c))).await.unwrap_or(false) {
            return;
        }
        let c = w.core.clone();
        let providers = match blocking(move || c.image_search_config()).await {
            Ok((cfg, key)) => providers_for(&cfg, key),
            Err(_) => vec![],
        };
        let worked = if providers.is_empty() {
            false
        } else {
            match process_one(&w.core, &providers, &fetcher).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(error = %e.message, "product image worker");
                    false
                }
            }
        };
        let pause = if worked { BETWEEN_JOBS } else { IDLE };
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = w.poke.notified() => { tokio::time::sleep(Duration::from_millis(300)).await; }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_and_internal_addresses_are_not_public() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.64.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "::ffff:192.168.0.1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} must be refused");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2a00:1450:4001::200e", "151.101.1.140"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} is public");
        }
    }

    #[tokio::test]
    async fn the_fetcher_refuses_unsafe_urls_before_connecting() {
        let f = SafeFetcher::default();
        for u in [
            "http://127.0.0.1/a.png",
            "http://localhost/a.png",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/a.png",
            "http://10.0.0.5/a.png",
            "http://192.168.1.10:8080/a.png",
            "file:///etc/passwd",
            "ftp://example.com/a.png",
            "gopher://example.com/",
            "https://user:pw@example.com/a.png",
            "http://metadata.google.internal/",
            "http://printer.local/a.png",
            "http://example.com:22/a.png",
            "not a url",
        ] {
            let e = f.check(u).await.unwrap_err();
            assert!(!e.transient, "{u}: {}", e.message);
        }
    }

    fn cand(title: &str, barcode_match: bool, w: Option<u32>) -> Candidate {
        Candidate {
            url: "https://cdn.example.com/p.jpg".into(),
            title: title.into(),
            page_url: Some("https://shop.example.bh/p".into()),
            width: w,
            height: w,
            barcode_match,
            provider: "test",
        }
    }

    fn q() -> SearchQuery {
        SearchQuery {
            barcode: Some("6281007031126".into()),
            name: "Almarai Fresh Milk 1L".into(),
            name_ar: None,
            category: None,
            text: "6281007031126 Almarai Fresh Milk 1L".into(),
            region: "bh".into(),
            language: "en".into(),
        }
    }

    #[test]
    fn bing_results_are_parsed_from_the_escaped_m_attribute() {
        let html = r##"<div class="imgpt"><a class="iusc" m="{&quot;purl&quot;:&quot;https://shop.example.bh/p/6281007031126&quot;,&quot;murl&quot;:&quot;https://cdn.example.com/milk.jpg?a=1&amp;b=2&quot;,&quot;t&quot;:&quot;Almarai Fresh Milk 1L&quot;}" href="#">x</a></div>
            <a class="iusc" m="{&quot;murl&quot;:&quot;https://cdn.example.com/milk.jpg?a=1&amp;b=2&quot;}">dup</a>
            <a class="iusc" m="{&quot;purl&quot;:&quot;https://blog.example.com&quot;,&quot;murl&quot;:&quot;https://img.example.com/logo.png&quot;,&quot;t&quot;:&quot;Almarai logo&quot;}">y</a>
            <a class="iusc" m="{broken json}">z</a>"##;
        let c = parse_bing(html, Some("6281007031126"));
        assert_eq!(c.len(), 2, "duplicates and broken entries skipped");
        assert_eq!(c[0].url, "https://cdn.example.com/milk.jpg?a=1&b=2");
        assert!(c[0].barcode_match, "barcode found on the page URL");
        assert_eq!(c[0].page_url.as_deref(), Some("https://shop.example.bh/p/6281007031126"));
        assert!(text_score(&c[1], &q()).is_none(), "logo refused");
        assert!(parse_bing("<html>no results</html>", None).is_empty());
    }

    #[test]
    fn text_scoring_needs_identity_and_skips_logos_and_thumbnails() {
        assert!(text_score(&cand("Almarai Fresh Milk 1 litre", false, Some(800)), &q()).is_some());
        assert!(text_score(&cand("Almarai logo vector", false, Some(800)), &q()).is_none(), "logo");
        assert!(text_score(&cand("Something unrelated", false, Some(800)), &q()).is_none(), "no identity");
        assert!(text_score(&cand("Something unrelated", true, Some(800)), &q()).is_some(), "barcode evidence");
        assert!(text_score(&cand("Almarai Fresh Milk", false, Some(120)), &q()).is_none(), "thumbnail");
        let gcc = text_score(&cand("Almarai Fresh Milk", false, Some(800)), &q()).unwrap();
        let mut other = cand("Almarai Fresh Milk", false, Some(800));
        other.page_url = Some("https://shop.example.com/p".into());
        assert!(gcc > text_score(&other, &q()).unwrap(), "GCC market is a small boost");
        let job = AutoImageJob {
            product_id: "p".into(),
            name: "Laban".into(),
            name_ar: None,
            sku: "100001".into(),
            barcodes: vec!["12".into(), "6281007031126".into()],
            category: Some("Dairy".into()),
            attempt: 1,
        };
        assert_eq!(build_query(&job, &ImageSearchSettings::default()).text, "6281007031126 Laban");
        let job = AutoImageJob { barcodes: vec![], ..job };
        assert_eq!(build_query(&job, &ImageSearchSettings::default()).text, "Laban Dairy", "no barcode is invented");
    }
}

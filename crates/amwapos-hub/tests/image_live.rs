//! Live provider smoke test: NOT part of CI or any normal test run (every
//! test here is `#[ignore]`, and the workspace sets `TILLBH_IMAGE_SEARCH=off`
//! for cargo processes anyway; these tests call the providers directly).
//!
//! Run by hand, with internet access, to check that the real sources still
//! honour the contract the worker relies on:
//!
//! ```text
//! cargo test -p amwapos-hub --test image_live -- --ignored --nocapture
//! ```
//!
//! A failure means the provider changed (Bing's unofficial results page in
//! particular) or is unreachable from this network; the application itself
//! does not depend on this test. The search clients use the system proxy.
//! The download test uses the production download policy (direct, or
//! `TILLBH_IMAGE_FETCH_PROXY`).

use amwapos_hub::image_worker::{BingImages, ImageSearchProvider, OpenFoodFacts, SafeFetcher, SearchQuery};

/// Coca-Cola Original 330 ml can: a long-lived, widely listed product.
const BARCODE: &str = "5449000000996";

fn query(barcode: Option<&str>, text: &str) -> SearchQuery {
    SearchQuery {
        barcode: barcode.map(String::from),
        name: "Coca-Cola Original 330ml".into(),
        name_ar: None,
        category: None,
        text: text.into(),
        region: "bh".into(),
        language: "en".into(),
    }
}

#[tokio::test]
#[ignore = "live network; run by hand"]
async fn live_bing_images_still_returns_parseable_candidates() {
    let bing = BingImages::new("https://www.bing.com");
    let hits = bing
        .search(&query(None, "Coca-Cola Original 330ml can"))
        .await
        .unwrap_or_else(|e| panic!("Bing images: {} (transient={}) — endpoint unreachable or format changed", e.message, e.transient));
    println!("bing: {} candidates; first: {:?}", hits.len(), hits.first().map(|c| &c.url));
    assert!(!hits.is_empty(), "Bing answered but no candidate could be extracted");
    assert!(hits.iter().all(|c| c.url.starts_with("http://") || c.url.starts_with("https://")));
}

#[tokio::test]
#[ignore = "live network; run by hand"]
async fn live_open_food_facts_still_answers_by_barcode() {
    let off = OpenFoodFacts::new("https://world.openfoodfacts.org");
    let hits = off.search(&query(Some(BARCODE), BARCODE)).await.unwrap_or_else(|e| panic!("Open Food Facts: {}", e.message));
    println!("open food facts: {} candidates; first: {:?}", hits.len(), hits.first().map(|c| &c.url));
    assert!(!hits.is_empty() && hits.iter().all(|c| c.barcode_match), "no barcode-tied image for {BARCODE}");
}

#[tokio::test]
#[ignore = "live network; run by hand"]
async fn live_candidate_downloads_through_the_production_fetcher() {
    let off = OpenFoodFacts::new("https://world.openfoodfacts.org");
    let hits = off.search(&query(Some(BARCODE), BARCODE)).await.expect("search");
    let url = &hits.first().expect("a candidate").url;
    let bytes = SafeFetcher::default().fetch(url).await.unwrap_or_else(|e| panic!("download {url}: {}", e.message));
    let n = amwapos_core::product_images::normalize(&bytes).expect("a decodable image");
    println!("downloaded {} bytes → {}×{} (white border {:.2})", bytes.len(), n.width, n.height, n.white_border);
}

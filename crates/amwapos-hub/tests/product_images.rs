//! Product image worker end to end against a local fixture server standing in
//! for the image search source and the image hosts: a found image is copied
//! into managed storage (the source can then disappear), a second pass does
//! nothing, no result / bad candidates end in not_found, provider outages are
//! retried (bounded), and the SSRF guard refuses internal addresses.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use amwapos_core::catalog::{ProductCreate, ProductInput};
use amwapos_core::product_images::ImageSearchSettings;
use amwapos_core::service::{AppCore, MemorySecretStore};
use amwapos_hub::image_worker::{process_one, BingImages, ImageSearchProvider, OpenFoodFacts, SafeFetcher};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::json;

#[derive(Clone)]
struct Fixture {
    base: String,
    hits: Arc<AtomicUsize>,
    image_hits: Arc<AtomicUsize>,
}

fn packshot() -> Vec<u8> {
    let mut img = image::RgbaImage::from_pixel(600, 600, image::Rgba([255, 255, 255, 255]));
    for y in 150..450 {
        for x in 200..400 {
            img.put_pixel(x, y, image::Rgba([180, 30, 30, 255]));
        }
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

async fn product(State(f): State<Fixture>, Path(code): Path<String>) -> Response {
    f.hits.fetch_add(1, Ordering::SeqCst);
    match code.as_str() {
        "6281007031126" => axum::Json(json!({ "status": 1, "code": code, "product": {
            "product_name": "Fresh Milk", "brands": "Almarai",
            "image_front_url": format!("{}/img/milk.png", f.base) } }))
        .into_response(),
        // A hit whose image is not an image, and one pointing inside the network.
        "6281007031127" => axum::Json(json!({ "status": 1, "code": code, "product": {
            "product_name": "Bad", "image_front_url": format!("{}/img/html", f.base),
            "image_url": "http://169.254.169.254/latest/meta-data/" } }))
        .into_response(),
        "5000000000005" => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        _ => axum::Json(json!({ "status": 0, "code": code })).into_response(),
    }
}

fn dark_photo() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(700, 500, image::Rgba([40, 60, 30, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Bing's results fragment: a logo, a lifestyle photo (dark, no white frame)
/// and a white packshot, all "about" the product.
async fn bing(State(f): State<Fixture>) -> Response {
    f.hits.fetch_add(1, Ordering::SeqCst);
    let m = |murl: String, t: &str| {
        format!(
            r#"<a class="iusc" m="{{&quot;purl&quot;:&quot;https://shop.example.bh/kiri&quot;,&quot;murl&quot;:&quot;{murl}&quot;,&quot;t&quot;:&quot;{t}&quot;}}">x</a>"#
        )
    };
    let html = [
        m(format!("{}/img/milk.png", f.base), "Kiri logo vector"),
        m(format!("{}/img/dark.png", f.base), "Kiri Cream Cheese 12 portions breakfast"),
        m(format!("{}/img/milk.png?v=packshot", f.base), "Kiri Cream Cheese 12 Portions 216g"),
    ]
    .join("\n");
    ([(header::CONTENT_TYPE, "text/html")], html).into_response()
}

async fn img(State(f): State<Fixture>, Path(name): Path<String>) -> Response {
    f.image_hits.fetch_add(1, Ordering::SeqCst);
    match name.as_str() {
        "milk.png" => ([(header::CONTENT_TYPE, "image/png")], packshot()).into_response(),
        "dark.png" => ([(header::CONTENT_TYPE, "image/png")], dark_photo()).into_response(),
        "loop" => (StatusCode::FOUND, [(header::LOCATION, "/img/loop")]).into_response(),
        "huge" => ([(header::CONTENT_TYPE, "image/png")], vec![0u8; 9 * 1024 * 1024]).into_response(),
        _ => ([(header::CONTENT_TYPE, "text/html")], "<html>not an image</html>").into_response(),
    }
}

async fn fixture() -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let f = Fixture { base: format!("http://{addr}"), hits: Arc::new(AtomicUsize::new(0)), image_hits: Arc::new(AtomicUsize::new(0)) };
    let app = Router::new()
        .route("/api/v2/product/{code}", get(product))
        .route("/img/{name}", get(img))
        .route("/images/async", get(bing))
        .with_state(f.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    f
}

struct Env {
    _dir: tempfile::TempDir,
    core: Arc<AppCore>,
    t: String,
    tax: String,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let core = Arc::new(AppCore::open(dir.path(), Arc::new(MemorySecretStore::default())).unwrap());
    core.setup_initialize(
        serde_json::from_value(json!({ "business_name": "Test Mart", "branch_name": "Main", "vat_rate_bp": 1000,
            "owner_name": "Owner", "owner_pin": "4826", "device_name": "Till", "device_code": "T01" }))
        .unwrap(),
    )
    .unwrap();
    let owner = core.login_users().unwrap().into_iter().next().unwrap().user_id;
    let t = core.login(&owner, "4826").unwrap().token;
    let mut f = core.settings_get(&t, "features").unwrap();
    f["catalog.auto_images"] = json!(true);
    core.settings_save(&t, "features", f).unwrap();
    let tax = core.tax_rules_list(&t).unwrap()[0].tax_rule_id.clone();
    Env { _dir: dir, core, t, tax }
}

fn create(e: &Env, name: &str, barcode: &str) -> String {
    e.core
        .product_create(
            &e.t,
            ProductCreate {
                product: ProductInput {
                    sku: None,
                    name: name.into(),
                    name_ar: None,
                    description: None,
                    category_id: None,
                    tax_rule_id: e.tax.clone(),
                    unit: "pcs".into(),
                    track_inventory: false,
                    allow_decimal_quantity: false,
                    reorder_point_milli: 0,
                    is_favorite: false,
                },
                price_minor: 450,
                cost_minor: None,
                barcodes: vec![barcode.into()],
                opening_stock_milli: None,
                image_b64: None,
            },
        )
        .unwrap()
        .row
        .product_id
}

fn status(e: &Env, pid: &str) -> (String, Option<String>, Option<String>) {
    let d = e.core.product_get(&e.t, pid).unwrap();
    (d.row.auto_image_status, d.row.image_source, d.row.image_hash)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_found_image_is_stored_once_and_survives_the_source_going_away() {
    let fx = fixture().await;
    let e = env();
    let providers: Vec<Arc<dyn ImageSearchProvider>> = vec![Arc::new(OpenFoodFacts::new(&fx.base))];
    let fetcher = SafeFetcher::allowing_private_for_tests();
    let pid = create(&e, "Almarai Fresh Milk 1L", "6281007031126");
    // Reading products never searches.
    e.core.product_get(&e.t, &pid).unwrap();
    e.core.products_search(&e.t, Default::default()).unwrap();
    assert_eq!(fx.hits.load(Ordering::SeqCst), 0);
    assert!(process_one(&e.core, &providers, &fetcher).await.unwrap());
    let (st, src, hash) = status(&e, &pid);
    assert_eq!((st.as_str(), src.as_deref()), ("found", Some("automatic")));
    let hash = hash.unwrap();
    // Managed copy: served from the database, not the source URL.
    let imgs = e.core.product_images_get(&e.t, vec![hash.clone()]).unwrap();
    assert!(imgs[&hash].starts_with("data:image/jpeg;base64,"));
    let d = e.core.product_image_state(&e.t, &pid).unwrap();
    let note = d.auto_image_note.unwrap();
    assert_eq!(note["provider"], "open_food_facts");
    assert_eq!(note["barcode_match"], true);
    // Nothing is due any more: no second search or download.
    let (h, i) = (fx.hits.load(Ordering::SeqCst), fx.image_hits.load(Ordering::SeqCst));
    assert!(!process_one(&e.core, &providers, &fetcher).await.unwrap());
    assert_eq!((fx.hits.load(Ordering::SeqCst), fx.image_hits.load(Ordering::SeqCst)), (h, i));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_result_and_unusable_candidates_end_in_not_found_and_outages_retry() {
    let fx = fixture().await;
    let e = env();
    let providers: Vec<Arc<dyn ImageSearchProvider>> = vec![Arc::new(OpenFoodFacts::new(&fx.base))];
    let fetcher = SafeFetcher::allowing_private_for_tests();
    let unknown = create(&e, "Mystery Snack", "6200000000001");
    assert!(process_one(&e.core, &providers, &fetcher).await.unwrap());
    assert_eq!(status(&e, &unknown).0, "not_found");
    // Candidates that are HTML or point at the metadata service are refused.
    let bad = create(&e, "Bad Candidates", "6281007031127");
    assert!(process_one(&e.core, &providers, &fetcher).await.unwrap());
    let (st, _, hash) = status(&e, &bad);
    assert_eq!((st.as_str(), hash), ("not_found", None));
    // Provider down: back in the queue with a delay, not failed, not repeated at once.
    let down = create(&e, "Outage", "5000000000005");
    assert!(process_one(&e.core, &providers, &fetcher).await.unwrap());
    assert_eq!(status(&e, &down).0, "pending");
    let hits = fx.hits.load(Ordering::SeqCst);
    assert!(!process_one(&e.core, &providers, &fetcher).await.unwrap());
    assert_eq!(fx.hits.load(Ordering::SeqCst), hits, "waits for its retry time");
    // Products without a usable barcode are not looked up by barcode.
    let nob = create(&e, "Loose Tomatoes", "12");
    assert!(process_one(&e.core, &providers, &fetcher).await.unwrap());
    assert_eq!(status(&e, &nob).0, "not_found");
    assert_eq!(fx.hits.load(Ordering::SeqCst), hits);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_production_fetcher_refuses_internal_hosts_redirect_loops_and_oversized_bodies() {
    let fx = fixture().await;
    let strict = SafeFetcher::default();
    let e = strict.fetch(&format!("{}/img/milk.png", fx.base)).await.unwrap_err();
    assert!(!e.transient && (e.message.contains("private") || e.message.contains("port")), "{}", e.message);
    let e = strict.fetch("http://127.0.0.1/img/milk.png").await.unwrap_err();
    assert!(!e.transient && e.message.contains("private"), "{}", e.message);
    assert_eq!(fx.image_hits.load(Ordering::SeqCst), 0, "never connected");
    let lenient = SafeFetcher::allowing_private_for_tests();
    assert!(lenient.fetch(&format!("{}/img/loop", fx.base)).await.unwrap_err().message.contains("redirects"));
    assert!(lenient.fetch(&format!("{}/img/huge", fx.base)).await.unwrap_err().message.contains("too large"));
    assert!(lenient.fetch(&format!("{}/img/page", fx.base)).await.unwrap_err().message.contains("not an image"));
    assert!(!lenient.fetch(&format!("{}/img/milk.png", fx.base)).await.unwrap().is_empty());
    let _ = ImageSearchSettings::default();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bing_results_are_used_when_the_barcode_source_has_nothing_and_a_white_packshot_wins() {
    let fx = fixture().await;
    let e = env();
    let providers: Vec<Arc<dyn ImageSearchProvider>> = vec![Arc::new(OpenFoodFacts::new(&fx.base)), Arc::new(BingImages::new(&fx.base))];
    let fetcher = SafeFetcher::allowing_private_for_tests();
    let pid = create(&e, "Kiri Cream Cheese 12 portions", "3073781037025");
    assert!(process_one(&e.core, &providers, &fetcher).await.unwrap());
    let (st, src, hash) = status(&e, &pid);
    assert_eq!((st.as_str(), src.as_deref()), ("found", Some("automatic")));
    assert!(hash.is_some());
    let note = e.core.product_image_state(&e.t, &pid).unwrap().auto_image_note.unwrap();
    assert_eq!(note["provider"], "bing_images");
    assert!(note["white_background"].as_f64().unwrap() > 0.9, "the white packshot, not the dark photo: {note}");
    assert_eq!(note["query"], "3073781037025 Kiri Cream Cheese 12 portions");
}

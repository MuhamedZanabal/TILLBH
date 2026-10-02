//! Image downloads and proxies. Behind a forward proxy the proxy resolves the
//! host again, so the fetcher's address check would no longer bind the real
//! destination. Policy: downloads never use the system proxy
//! (`HTTP(S)_PROXY`); they connect directly to the checked, pinned address.
//! Only a proxy an administrator names in `TILLBH_IMAGE_FETCH_PROXY` is used,
//! and the local destination check still runs first.
//! (One test in this binary: it sets process-wide proxy variables.)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tillbh_hub::image_worker::{ProxyPolicy, SafeFetcher};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(80, 80, image::Rgba([255, 255, 255, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A server counting its requests; used both as an image host and as a
/// forward proxy (a proxy receives the absolute URL; axum routes it all to
/// the fallback).
async fn counting_server() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let app = Router::new().fallback(get(move || {
        let h = h.clone();
        async move {
            h.fetch_add(1, Ordering::SeqCst);
            ([(header::CONTENT_TYPE, "image/png")], png()).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), hits)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_ignore_the_system_proxy_and_use_only_an_explicitly_trusted_one() {
    let (host, host_hits) = counting_server().await;
    let (proxy, proxy_hits) = counting_server().await;
    for k in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        std::env::set_var(k, &proxy);
    }
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");
    std::env::remove_var("TILLBH_IMAGE_FETCH_PROXY");

    // Default policy: direct. The system proxy is never contacted.
    assert_eq!(SafeFetcher::default().proxy_policy(), &ProxyPolicy::Direct);
    let direct = SafeFetcher::allowing_private_for_tests();
    assert!(!direct.fetch(&format!("{host}/a.png")).await.unwrap().is_empty());
    assert_eq!((host_hits.load(Ordering::SeqCst), proxy_hits.load(Ordering::SeqCst)), (1, 0));

    // The production fetcher refuses internal destinations before any
    // connection, whether or not a proxy is configured or trusted.
    for policy in [ProxyPolicy::Direct, ProxyPolicy::Trusted(proxy.clone())] {
        let strict = SafeFetcher::default().with_proxy(policy);
        for u in ["http://10.0.0.7/a.png", "http://169.254.169.254/latest/meta-data/", "http://[fd00::1]/a.png", &format!("{host}/a.png")] {
            let e = strict.fetch(u).await.unwrap_err();
            assert!(!e.transient, "{u}: {}", e.message);
        }
    }
    assert_eq!((host_hits.load(Ordering::SeqCst), proxy_hits.load(Ordering::SeqCst)), (1, 0), "nothing was contacted");

    // Explicitly trusted proxy: named by the administrator, then used.
    std::env::set_var("TILLBH_IMAGE_FETCH_PROXY", &proxy);
    assert_eq!(SafeFetcher::default().proxy_policy(), &ProxyPolicy::Trusted(proxy.clone()));
    let trusted = SafeFetcher::allowing_private_for_tests().with_proxy(ProxyPolicy::Trusted(proxy.clone()));
    assert!(!trusted.fetch(&format!("{host}/b.png")).await.unwrap().is_empty());
    assert_eq!((host_hits.load(Ordering::SeqCst), proxy_hits.load(Ordering::SeqCst)), (1, 1), "went through the trusted proxy");
    // A malformed trusted proxy address is refused cleanly.
    let bad = SafeFetcher::allowing_private_for_tests().with_proxy(ProxyPolicy::Trusted("not a url".into()));
    assert!(bad.fetch(&format!("{host}/c.png")).await.is_err());
}

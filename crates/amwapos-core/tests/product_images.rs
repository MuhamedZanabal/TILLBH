//! Product images: manual upload / replace / remove, precedence over the
//! automatic image, and the one-time automatic lookup state machine.
mod common;

use std::io::Cursor;

use amwapos_core::auth::ROLE_CASHIER;
use amwapos_core::catalog::{ProductCreate, ProductInput};
use amwapos_core::product_images::{normalize, AutoOutcome};
use amwapos_core::ErrorCode;
use base64::Engine;
use common::*;
use serde_json::json;

fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
    for y in h / 4..h * 3 / 4 {
        for x in w / 4..w * 3 / 4 {
            img.put_pixel(x, y, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
        }
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn count(e: &Env, sql: &str) -> i64 {
    e.core.db.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
}

fn features(e: &Env, on: bool) {
    let mut f = e.core.settings_get(&e.owner_token, "features").unwrap();
    f["catalog.auto_images"] = json!(on);
    e.core.settings_save(&e.owner_token, "features", f).unwrap();
}

fn create(e: &Env, name: &str, barcode: &str, image: Option<&[u8]>) -> amwapos_core::catalog::ProductDetail {
    e.core
        .product_create(
            &e.owner_token,
            ProductCreate {
                product: ProductInput {
                    sku: None,
                    name: name.into(),
                    name_ar: None,
                    description: None,
                    category_id: None,
                    tax_rule_id: e.tax_rule(),
                    unit: "pcs".into(),
                    track_inventory: true,
                    allow_decimal_quantity: false,
                    reorder_point_milli: 0,
                    is_favorite: false,
                },
                price_minor: 500,
                cost_minor: None,
                barcodes: vec![barcode.into()],
                opening_stock_milli: None,
                image_b64: image.map(b64),
            },
        )
        .unwrap()
}

#[test]
fn manual_image_on_create_replace_remove_and_garbage_collection() {
    let e = env();
    features(&e, true);
    let red = png(400, 400, [200, 20, 20]);
    let p = create(&e, "Almarai Laban 1L", "6281007031126", Some(&red));
    assert_eq!(p.row.image_source.as_deref(), Some("manual"));
    assert_eq!(p.row.auto_image_status, "skipped", "a manual image means no automatic lookup");
    let h1 = p.row.image_hash.clone().unwrap();
    assert_eq!(count(&e, "SELECT COUNT(*) FROM product_images"), 1);
    // Served to any signed-in screen as a data URL; the cashier can read it too.
    let (_u, ct) = e.user("Cashier", ROLE_CASHIER, "2580");
    let imgs = e.core.product_images_get(&ct, vec![h1.clone(), "0".repeat(64)]).unwrap();
    assert!(imgs[&h1].starts_with("data:image/jpeg;base64,"));
    assert_eq!(imgs.len(), 1);
    // The worker never takes it.
    assert!(e.core.auto_image_claim().unwrap().is_none());
    // Replace: the new image is stored, the old one collected.
    let blue = png(300, 500, [20, 20, 200]);
    let st = e.core.product_image_upload(&e.owner_token, &p.row.product_id, &b64(&blue)).unwrap();
    let h2 = st.image_hash.clone().unwrap();
    assert_ne!(h1, h2);
    assert_eq!(count(&e, "SELECT COUNT(*) FROM product_images"), 1);
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM product_images WHERE image_hash='{h2}'")), 1);
    // A second product with the same picture shares the stored copy; removing
    // it from one product keeps it for the other.
    let q = create(&e, "Almarai Laban 2L", "6281007031127", Some(&blue));
    assert_eq!(q.row.image_hash.as_deref(), Some(h2.as_str()));
    let st = e.core.product_image_remove(&e.owner_token, &p.row.product_id).unwrap();
    assert!(st.image_hash.is_none() && st.image_source.is_none());
    assert_eq!(st.auto_image_status, "skipped", "removing never re-queues the lookup");
    assert_eq!(count(&e, "SELECT COUNT(*) FROM product_images"), 1, "still used by the second product");
    e.core.product_image_remove(&e.owner_token, &q.row.product_id).unwrap();
    assert_eq!(count(&e, "SELECT COUNT(*) FROM product_images"), 0);
    assert!(e.core.auto_image_claim().unwrap().is_none());
    assert!(count(&e, "SELECT COUNT(*) FROM audit_logs WHERE event_type IN ('product.image_set','product.image_removed')") >= 3);
}

#[test]
fn invalid_uploads_are_refused_and_nothing_is_created() {
    let e = env();
    let before = count(&e, "SELECT COUNT(*) FROM products");
    for bad in [b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(), b"GIF89a-broken".to_vec(), vec![0xFF, 0xD8, 0xFF, 0x00]] {
        let r = e.core.product_create(
            &e.owner_token,
            ProductCreate {
                product: serde_json::from_value(json!({ "name": "X", "tax_rule_id": e.tax_rule() })).unwrap(),
                price_minor: 1,
                cost_minor: None,
                barcodes: vec![],
                opening_stock_milli: None,
                image_b64: Some(b64(&bad)),
            },
        );
        assert_eq!(r.unwrap_err().code, ErrorCode::Validation);
    }
    assert_eq!(count(&e, "SELECT COUNT(*) FROM products"), before, "a bad picture saves nothing");
    let p = create(&e, "Kiri 12", "3073781037025", None);
    let err = e.core.product_image_upload(&e.owner_token, &p.row.product_id, "not-base64!!").unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    let err = e.core.product_image_upload(&e.owner_token, &p.row.product_id, &b64(b"%PDF-1.7")).unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation);
    // A cashier cannot change product images.
    let (_u, ct) = e.user("Cashier", ROLE_CASHIER, "2580");
    let err = e.core.product_image_upload(&ct, &p.row.product_id, &b64(&png(100, 100, [1, 2, 3]))).unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    assert_eq!(e.core.product_image_remove(&ct, &p.row.product_id).unwrap_err().code, ErrorCode::Forbidden);
    // Unknown product.
    let err = e.core.product_image_upload(&e.owner_token, "01ARZ3NDEKTSV4RRFFQ69G5FAV", &b64(&png(100, 100, [1, 2, 3]))).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn automatic_lookup_runs_once_and_its_result_persists() {
    let e = env();
    // Off: products are not queued (and legacy/import rows stay not_attempted).
    let off = create(&e, "Before switch", "111", None);
    assert_eq!(off.row.auto_image_status, "not_attempted");
    features(&e, true);
    let p = create(&e, "Nido Milk Powder 900g", "7613035123456", None);
    assert_eq!(p.row.auto_image_status, "pending");
    // Claim: exactly one worker gets it; reading or editing never triggers anything.
    let job = e.core.auto_image_claim().unwrap().expect("queued");
    assert_eq!((job.product_id.as_str(), job.barcodes[0].as_str()), (p.row.product_id.as_str(), "7613035123456"));
    assert!(e.core.auto_image_claim().unwrap().is_none(), "no double claim");
    e.core.product_get(&e.owner_token, &p.row.product_id).unwrap();
    e.core.products_search(&e.owner_token, Default::default()).unwrap();
    assert_eq!(count(&e, &format!("SELECT auto_image_attempts FROM products WHERE product_id='{}'", p.row.product_id)), 1);
    // Found: stored copy, automatic provenance, terminal.
    let img = normalize(&png(600, 600, [30, 120, 30])).unwrap();
    let hash = img.hash.clone();
    let state =
        e.core.auto_image_complete(&job.product_id, AutoOutcome::Found { image: img, note: json!({ "provider": "test" }) }).unwrap();
    assert_eq!(state, "found");
    let d = e.core.product_get(&e.owner_token, &p.row.product_id).unwrap();
    assert_eq!(
        (d.row.image_hash.as_deref(), d.row.image_source.as_deref(), d.row.auto_image_status.as_str()),
        (Some(hash.as_str()), Some("automatic"), "found")
    );
    assert!(e.core.auto_image_claim().unwrap().is_none(), "never again");
    // Editing name/barcode does not re-run it.
    let upd = e.core.product_update(
        &e.owner_token,
        serde_json::from_value(json!({ "product_id": p.row.product_id, "expected_version": d.version,
            "name": "Nido Fortified 900g", "tax_rule_id": e.tax_rule() }))
        .unwrap(),
    );
    assert_eq!(upd.unwrap().row.image_hash.as_deref(), Some(hash.as_str()), "automatic image kept on edit");
    assert!(e.core.auto_image_claim().unwrap().is_none());
    // A manual upload replaces the automatic image and becomes authoritative.
    let st = e.core.product_image_upload(&e.owner_token, &p.row.product_id, &b64(&png(200, 200, [9, 9, 9]))).unwrap();
    assert_eq!(st.image_source.as_deref(), Some("manual"));
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM product_images WHERE image_hash='{hash}'")), 0, "old automatic copy collected");
}

#[test]
fn not_found_and_failures_are_terminal_and_transient_errors_retry_boundedly() {
    let e = env();
    features(&e, true);
    let a = create(&e, "Unknown thing", "999000111", None);
    let j = e.core.auto_image_claim().unwrap().unwrap();
    assert_eq!(
        e.core.auto_image_complete(&j.product_id, AutoOutcome::NotFound { note: json!({ "reason": "none" }) }).unwrap(),
        "not_found"
    );
    assert!(e.core.auto_image_claim().unwrap().is_none(), "not_found never retries");
    assert_eq!(e.core.product_get(&e.owner_token, &a.row.product_id).unwrap().row.auto_image_status, "not_found");
    // Transient: back to pending with a delay, then failed after the cap.
    let b = create(&e, "Flaky", "999000222", None);
    let j = e.core.auto_image_claim().unwrap().unwrap();
    assert_eq!(j.product_id, b.row.product_id);
    assert_eq!(e.core.auto_image_complete(&j.product_id, AutoOutcome::Transient { note: json!({ "error": "timeout" }) }).unwrap(), "retry");
    assert!(e.core.auto_image_claim().unwrap().is_none(), "waits for its retry time");
    for _ in 0..2 {
        e.core.db.write(|c| Ok(c.execute("UPDATE products SET auto_image_next_at=NULL", [])?)).unwrap();
        let j = e.core.auto_image_claim().unwrap().unwrap();
        e.core.auto_image_complete(&j.product_id, AutoOutcome::Transient { note: json!({ "error": "503" }) }).unwrap();
    }
    assert_eq!(e.core.product_get(&e.owner_token, &b.row.product_id).unwrap().row.auto_image_status, "failed");
    e.core.db.write(|c| Ok(c.execute("UPDATE products SET auto_image_next_at=NULL", [])?)).unwrap();
    assert!(e.core.auto_image_claim().unwrap().is_none(), "failed is terminal");
    // Explicit, user-triggered search is the only way back.
    let st = e.core.product_image_find(&e.owner_token, &b.row.product_id).unwrap();
    assert_eq!(st.auto_image_status, "pending");
    assert!(e.core.auto_image_claim().unwrap().is_some());
}

#[test]
fn a_manual_upload_during_a_lookup_wins_and_a_stale_claim_is_recovered() {
    let e = env();
    features(&e, true);
    let p = create(&e, "Race", "999000333", None);
    let j = e.core.auto_image_claim().unwrap().unwrap();
    e.core.product_image_upload(&e.owner_token, &p.row.product_id, &b64(&png(120, 120, [1, 2, 3]))).unwrap();
    let found = normalize(&png(500, 500, [200, 200, 0])).unwrap();
    let fh = found.hash.clone();
    assert_eq!(e.core.auto_image_complete(&j.product_id, AutoOutcome::Found { image: found, note: json!({}) }).unwrap(), "superseded");
    let d = e.core.product_get(&e.owner_token, &p.row.product_id).unwrap();
    assert_eq!(d.row.image_source.as_deref(), Some("manual"));
    assert_eq!(count(&e, &format!("SELECT COUNT(*) FROM product_images WHERE image_hash='{fh}'")), 0, "nothing orphaned");
    // A worker that died mid-lookup: its claim is taken back after the timeout.
    let q = create(&e, "Abandoned", "999000444", None);
    let _ = e.core.auto_image_claim().unwrap().unwrap();
    assert!(e.core.auto_image_claim().unwrap().is_none());
    e.core
        .db
        .write(|c| {
            Ok(c.execute(
                "UPDATE products SET auto_image_attempted_at='2000-01-01T00:00:00.000Z' WHERE product_id=?1",
                [&q.row.product_id],
            )?)
        })
        .unwrap();
    assert_eq!(e.core.auto_image_claim().unwrap().unwrap().product_id, q.row.product_id);
}

#[test]
fn backfill_is_explicit_bounded_and_idempotent() {
    let e = env();
    // Existing products (before the switch) are not searched on their own.
    for i in 0..5 {
        create(&e, &format!("Legacy {i}"), &format!("55500{i}"), None);
    }
    create(&e, "Has picture", "555999", Some(&png(100, 100, [5, 5, 5])));
    let off = e.core.product_image_backfill(&e.owner_token, None).unwrap_err();
    assert_eq!(off.details.unwrap()["kind"], "feature_disabled", "needs the switch");
    features(&e, true);
    assert!(e.core.auto_image_claim().unwrap().is_none(), "switching on alone searches nothing");
    let r = e.core.product_image_backfill(&e.owner_token, Some(3)).unwrap();
    assert_eq!(r["queued"], 3);
    let r = e.core.product_image_backfill(&e.owner_token, Some(10)).unwrap();
    assert_eq!(r["queued"], 2, "only the rest; the product with a picture is never queued");
    assert_eq!(e.core.product_image_backfill(&e.owner_token, None).unwrap()["queued"], 0);
    let o = e.core.product_image_overview(&e.owner_token).unwrap();
    assert_eq!(o["counts"]["pending"], 5);
    assert_eq!(o["google_key_set"], false);
    let (_u, ct) = e.user("Cashier", ROLE_CASHIER, "2580");
    assert_eq!(e.core.product_image_backfill(&ct, None).unwrap_err().code, ErrorCode::Forbidden);
}

#[test]
fn search_settings_and_key_stay_on_the_backend() {
    let e = env();
    let cfg = json!({ "open_food_facts": true, "google": true, "google_cx": "abc123:xyz", "region": "BH", "language": "ar" });
    let o = e.core.product_image_configure(&e.owner_token, serde_json::from_value(cfg).unwrap(), Some("AIzaSy-secret-key".into())).unwrap();
    assert_eq!(o["google_key_set"], true);
    assert_eq!(o["settings"]["region"], "bh");
    assert!(!o.to_string().contains("AIzaSy-secret-key"), "the key is never returned");
    let (cfg, key) = e.core.image_search_config().unwrap();
    assert_eq!((cfg.google_cx.as_str(), key.as_deref()), ("abc123:xyz", Some("AIzaSy-secret-key")));
    let bad = json!({ "region": "bahrain", "language": "en" });
    assert!(e.core.product_image_configure(&e.owner_token, serde_json::from_value(bad).unwrap(), None).is_err());
    let (_u, ct) = e.user("Cashier", ROLE_CASHIER, "2580");
    let any = json!({ "region": "bh", "language": "en" });
    assert_eq!(e.core.product_image_configure(&ct, serde_json::from_value(any).unwrap(), None).unwrap_err().code, ErrorCode::Forbidden);
}

#[test]
fn product_rows_pos_search_and_cart_carry_the_image_reference() {
    let e = env();
    let p = create(&e, "Galaxy 36g", "5000159461122", Some(&png(150, 150, [90, 40, 10])));
    let h = p.row.image_hash.clone().unwrap();
    let (_u, ct) = e.user("Cashier", ROLE_CASHIER, "2580");
    e.open_shift(&ct, 0);
    let rows = e.core.pos_search(&ct, "Galaxy", None, false, None).unwrap();
    assert_eq!(rows[0].image_hash.as_deref(), Some(h.as_str()));
    let cart = e.core.pos_scan(&ct, "5000159461122", Some(1000)).unwrap();
    assert_eq!(cart.cart.lines[0].image_hash.as_deref(), Some(h.as_str()));
    // Products without images still sell normally.
    let q = create(&e, "Plain", "5000159461123", None);
    assert!(q.row.image_hash.is_none());
    let cart = e.core.pos_scan(&ct, "5000159461123", Some(1000)).unwrap();
    assert!(cart.cart.lines.iter().any(|l| l.name == "Plain" && l.image_hash.is_none()));
}
